//! Claude Code: `~/.claude/projects/<encoded-cwd>/<session>.jsonl` transcripts,
//! hooks in `.claude/settings.local.json`, MCP in `.mcp.json`.

use crate::hookparse::{self, base_event, other_tool, shell_result, tool};
use crate::ingest::Acc;
use crate::plan::{self, HookEntry, hook_entries, install_hooks, json_edit, json_uninstall};
use crate::util::{
    self, head_lines, lines, mtime, path_inside, plain, s, summarize_input, text_of, ts,
};
use crate::{
    AgentAdapter, BriefRef, Capabilities, ConfigPlan, Detection, HookPlan, HookScope, HostEnv,
    IngestOpts, Inject, LaunchSpec, RepoCtx, SessionRef, Witness,
};
use fh_core::{
    AgentId, FileOp, HookEvent, HookEventKind, IngestedSession, Result, Role, ToolClass,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Claude Code adapter.
#[derive(Debug, Clone)]
pub struct ClaudeCode {
    host: HostEnv,
}

/// Directory name Claude Code uses for a working directory under `projects/`: every
/// character other than an ASCII letter or digit becomes `-` (two for characters
/// outside the Basic Multilingual Plane, matching JavaScript's UTF-16 replace).
/// The mapping is lossy, so discovery confirms each match against the `cwd` field.
pub fn encode_project_dir(path: &str) -> String {
    let p = path.strip_prefix(r"\\?\").unwrap_or(path);
    p.chars()
        .flat_map(|c| {
            let n = if c.is_ascii_alphanumeric() {
                0
            } else {
                c.len_utf16()
            };
            let keep = (n == 0).then_some(c);
            keep.into_iter().chain(std::iter::repeat_n('-', n))
        })
        .collect()
}

const EVENTS: [(&str, Option<&str>); 8] = [
    ("SessionStart", Some("startup|resume|clear|compact")),
    ("PreToolUse", Some("Bash")),
    (
        "PostToolUse",
        Some("Bash|Write|Edit|MultiEdit|NotebookEdit"),
    ),
    ("PostToolUseFailure", Some("Bash")),
    ("Stop", None),
    ("SubagentStop", None),
    ("PreCompact", None),
    ("SessionEnd", None),
];

impl ClaudeCode {
    /// Adapter looking at `host`.
    pub fn new(host: HostEnv) -> Self {
        ClaudeCode { host }
    }

    fn config_dir(&self) -> Result<PathBuf> {
        match self.host.var("CLAUDE_CONFIG_DIR") {
            Some(d) => Ok(PathBuf::from(d)),
            None => Ok(util::home(&self.host)?.join(".claude")),
        }
    }

    fn user_mcp_file(&self) -> Result<PathBuf> {
        match self.host.var("CLAUDE_CONFIG_DIR") {
            Some(d) => Ok(PathBuf::from(d).join(".claude.json")),
            None => Ok(util::home(&self.host)?.join(".claude.json")),
        }
    }

    fn settings_path(&self, repo: &RepoCtx, scope: HookScope) -> Result<PathBuf> {
        Ok(match scope {
            HookScope::Repo => repo.root.join(".claude").join("settings.local.json"),
            HookScope::User => self.config_dir()?.join("settings.json"),
        })
    }

    fn mcp_path(&self, repo: &RepoCtx, scope: HookScope) -> Result<PathBuf> {
        match scope {
            HookScope::Repo => Ok(repo.root.join(".mcp.json")),
            HookScope::User => self.user_mcp_file(),
        }
    }

    pub(crate) fn uninstall(&self, repo: &RepoCtx, scope: HookScope) -> Result<HookPlan> {
        let edits = [
            json_uninstall(self.settings_path(repo, scope)?, scope, None)?,
            json_uninstall(self.mcp_path(repo, scope)?, scope, Some("mcpServers"))?,
        ];
        Ok(HookPlan {
            agent: AgentId::ClaudeCode,
            edits: edits.into_iter().flatten().collect(),
            manual_steps: Vec::new(),
        })
    }
}

impl AgentAdapter for ClaudeCode {
    fn id(&self) -> AgentId {
        AgentId::ClaudeCode
    }

    fn detect(&self, host: &HostEnv) -> Detection {
        let me = ClaudeCode::new(host.clone());
        let mut configs = Vec::new();
        if let Ok(d) = me.config_dir() {
            configs.extend([d.join("settings.json"), d.join("projects")]);
        }
        configs.extend(me.user_mcp_file().ok());
        util::detect(host, "claude", configs)
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            witness_bash: Witness::Full,
            witness_edits: Witness::Full,
            pre_tool_events: true,
            session_start_inject: Inject::Stdout,
            stop_event: true,
            compact_event: true,
            subagent_events: true,
            ingest: true,
            mcp: true,
        }
    }

    fn discover_sessions(&self, repo: &RepoCtx) -> Result<Vec<SessionRef>> {
        let projects = self.config_dir()?.join("projects");
        let root = plain(&repo.root);
        let enc = encode_project_dir(&root);
        let Ok(dirs) = std::fs::read_dir(&projects) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for d in dirs.flatten() {
            let name = d.file_name().to_string_lossy().into_owned();
            // Claude Code shortens very long names and appends a hash.
            let candidate =
                name.starts_with(&enc) || (enc.len() > 200 && name.starts_with(&enc[..200]));
            if !candidate || !d.path().is_dir() {
                continue;
            }
            let Ok(files) = std::fs::read_dir(d.path()) else {
                continue;
            };
            for f in files.flatten() {
                let path = f.path();
                if path.extension().is_none_or(|e| e != "jsonl") {
                    continue;
                }
                let cwd = session_cwd(&path);
                let inside = match &cwd {
                    Some(c) => path_inside(c, &root),
                    None => name == enc,
                };
                if inside {
                    out.push(SessionRef {
                        agent: AgentId::ClaudeCode,
                        id: path
                            .file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                        modified: mtime(&path),
                        path,
                        cwd,
                    });
                }
            }
        }
        out.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    fn ingest(&self, sref: &SessionRef, opts: &IngestOpts) -> Result<IngestedSession> {
        let src = util::read_source(&sref.path, opts.max_bytes)?;
        let mut acc = Acc::default();
        for line in lines(&src.bytes) {
            match serde_json::from_str::<Value>(&line) {
                Ok(v) if v.is_object() => record(&mut acc, &v),
                _ => acc.skipped += 1,
            }
        }
        // Subagent transcripts: one child tool event each.
        let sub = sref.path.with_extension("").join("subagents");
        if let Ok(rd) = std::fs::read_dir(&sub) {
            let mut files: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
                .collect();
            files.sort();
            for f in files {
                let Ok(child) = util::read_source(&f, opts.max_bytes) else {
                    continue;
                };
                let mut c = Acc::default();
                for line in lines(&child.bytes) {
                    match serde_json::from_str::<Value>(&line) {
                        Ok(v) if v.is_object() => record(&mut c, &v),
                        _ => c.skipped += 1,
                    }
                }
                let child = c.finish(AgentId::ClaudeCode, "", child.hash, opts);
                let prompt = child
                    .messages
                    .iter()
                    .find(|m| m.role == Role::User)
                    .map(|m| util::cap(&m.text, 300))
                    .unwrap_or_default();
                let ts = child.messages.first().and_then(|m| m.ts.clone());
                let call = f.to_string_lossy().into_owned();
                acc.tool_use(Some(&call), "subagent", prompt, ts);
                acc.tool_result(Some(&call), &child.final_assistant_text, false);
                acc.skipped += child.skipped_records;
            }
        }
        Ok(acc.finish(AgentId::ClaudeCode, &sref.id, src.hash, opts))
    }

    fn hook_plan(&self, repo: &RepoCtx, scope: HookScope) -> Result<HookPlan> {
        let path = self.settings_path(repo, scope)?;
        let entries: Vec<HookEntry> = hook_entries(repo, AgentId::ClaudeCode, &EVENTS);
        let desc = format!("add firsthand hooks to {}", path.display());
        let edit = json_edit(path, scope, desc, |t| {
            install_hooks(t, &entries, &json!({}))
        })?;
        Ok(HookPlan {
            agent: AgentId::ClaudeCode,
            edits: edit.into_iter().collect(),
            manual_steps: vec![
                "Claude Code reads hooks when a session starts: restart open Claude Code sessions in this repo".into(),
            ],
        })
    }

    fn parse_hook_event(&self, raw: &[u8], argv_tail: &[String]) -> Result<HookEvent> {
        let v = hookparse::payload(raw, argv_tail)?;
        let mut ev = base_event(AgentId::ClaudeCode, &v, argv_tail);
        if let Some(name) = s(&v, "tool_name") {
            ev.tool = Some(claude_tool(name, &v, &ev));
        }
        Ok(ev)
    }

    fn mcp_install_plan(&self, repo: &RepoCtx, scope: HookScope) -> Result<ConfigPlan> {
        let path = self.mcp_path(repo, scope)?;
        let argv = plan::mcp_argv(repo);
        let entry = json!({"type": "stdio", "command": argv[0], "args": argv[1..]});
        let desc = format!("add the firsthand MCP server to {}", path.display());
        let edit = json_edit(path, scope, desc, |t| {
            crate::jsonedit::insert_member(t, &["mcpServers"], "firsthand", &entry)
        })?;
        let mut steps = Vec::new();
        if scope == HookScope::Repo {
            steps.push(
                "Claude Code asks once before it starts servers from .mcp.json: approve \"firsthand\"".into(),
            );
        }
        Ok(HookPlan {
            agent: AgentId::ClaudeCode,
            edits: edit.into_iter().collect(),
            manual_steps: steps,
        })
    }

    fn launch_spec(&self, brief: &BriefRef, repo: &RepoCtx) -> Result<LaunchSpec> {
        Ok(LaunchSpec {
            argv: vec!["claude".into(), brief.bootstrap.clone()],
            env: Vec::new(),
            cwd: repo.root.clone(),
            verified: false,
        })
    }
}

/// `cwd` from the first lines of a transcript.
fn session_cwd(path: &Path) -> Option<String> {
    head_lines(path, 50, 1024 * 1024).iter().find_map(|l| {
        let v: Value = serde_json::from_str(l).ok()?;
        s(&v, "cwd").map(str::to_owned)
    })
}

/// Fold one transcript line into the accumulator.
fn record(acc: &mut Acc, v: &Value) {
    let t = ts(v.get("timestamp"));
    if acc.session_id.is_none() {
        acc.session_id = s(v, "sessionId").map(str::to_owned);
    }
    if acc.cwd.is_none() {
        acc.cwd = s(v, "cwd").map(str::to_owned);
    }
    if let Some(b) = s(v, "gitBranch").filter(|b| !b.is_empty()) {
        acc.git_branch = Some(b.to_owned());
    }
    let kind = s(v, "type").unwrap_or("");
    if kind == "summary" {
        if let Some(text) = s(v, "summary") {
            acc.message(Role::System, text, t, true);
        }
        return;
    }
    if !matches!(kind, "user" | "assistant")
        || v.get("isMeta") == Some(&Value::Bool(true))
        || v.get("isSidechain") == Some(&Value::Bool(true))
    {
        return;
    }
    let Some(msg) = v.get("message") else {
        return;
    };
    let role = match s(msg, "role").unwrap_or(kind) {
        "assistant" => Role::Assistant,
        _ => Role::User,
    };
    let summary = v.get("isCompactSummary") == Some(&Value::Bool(true));
    let role = if summary { Role::System } else { role };
    match msg.get("content") {
        Some(Value::String(text)) => acc.message(role, text, t, summary),
        Some(Value::Array(blocks)) => {
            let mut text = Vec::new();
            for b in blocks {
                match s(b, "type") {
                    Some("text") => text.push(s(b, "text").unwrap_or("").to_owned()),
                    Some("tool_use") => {
                        let name = s(b, "name").unwrap_or("tool");
                        let input = b.get("input").unwrap_or(&Value::Null);
                        acc.tool_use(s(b, "id"), name, summarize_input(input), t.clone());
                    }
                    Some("tool_result") => {
                        let out = b.get("content").map(text_of).unwrap_or_default();
                        let err = b.get("is_error") == Some(&Value::Bool(true));
                        acc.tool_result(s(b, "tool_use_id"), &out, err);
                    }
                    // thinking, redacted_thinking, images and unknown blocks are never ingested.
                    _ => {}
                }
            }
            acc.message(role, &text.join("\n"), t, summary);
        }
        _ => {}
    }
}

/// Tool call of a Claude Code hook payload.
fn claude_tool(name: &str, v: &Value, ev: &HookEvent) -> fh_core::ToolCall {
    let input = v.get("tool_input").unwrap_or(&Value::Null);
    let failure = ev.raw_event_name == "PostToolUseFailure";
    match name {
        "Bash" | "PowerShell" => {
            let mut t = tool(name, ToolClass::Shell);
            (t.command, t.argv) = hookparse::shell_command(input);
            if failure {
                let err = s(v, "error").unwrap_or("");
                t.exit_code = hookparse::exit_code_in(err.lines().next().unwrap_or(""));
                t.stderr = Some(err.split_once('\n').map_or("", |(_, r)| r).to_owned());
                t.is_error = Some(true);
            } else if ev.kind == HookEventKind::PostToolUse {
                let resp = v.get("tool_response").unwrap_or(&Value::Null);
                let (code, out, err) = shell_result(resp);
                let interrupted = resp.get("interrupted") == Some(&Value::Bool(true));
                t.stdout = out;
                t.stderr = err;
                // PostToolUse fires only for calls that succeeded; Claude Code reports
                // no exit code for them, so success means 0 unless interrupted.
                t.exit_code = code.or((!interrupted).then_some(0));
                t.is_error = Some(interrupted);
            }
            t
        }
        "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
            let mut t = tool(name, ToolClass::FileEdit);
            t.file_paths = ["file_path", "notebook_path"]
                .iter()
                .filter_map(|k| s(input, k))
                .map(str::to_owned)
                .collect();
            t.file_op = Some(if name == "Write" {
                FileOp::Write
            } else {
                FileOp::Edit
            });
            if failure {
                t.is_error = Some(true);
            } else if ev.kind == HookEventKind::PostToolUse {
                t.is_error = Some(false);
            }
            t
        }
        _ => {
            let mut t = other_tool(name, input);
            if failure {
                t.is_error = Some(true);
            }
            t
        }
    }
}

//! Codex CLI: rollout JSONL under `~/.codex/sessions/YYYY/MM/DD/`, hooks in
//! `.codex/hooks.json`, MCP in `.codex/config.toml`.

use crate::hookparse::{self, base_event, other_tool, patch_files, shell_result, tool};
use crate::ingest::Acc;
use crate::plan::{self, hook_entries, install_hooks, json_edit, json_uninstall};
use crate::util::{
    self, head_lines, lines, mtime, path_inside, plain, read_opt, s, summarize_input, text_of, ts,
};
use crate::{
    AgentAdapter, BriefRef, Capabilities, ConfigPlan, Detection, FileEdit, HookPlan, HookScope,
    HostEnv, IngestOpts, Inject, LaunchSpec, RepoCtx, SessionRef, Witness,
};
use fh_core::{AgentId, Error, HookEvent, HookEventKind, IngestedSession, Result, Role, ToolClass};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Codex CLI adapter.
#[derive(Debug, Clone)]
pub struct Codex {
    host: HostEnv,
}

const EVENTS: [(&str, Option<&str>); 7] = [
    ("SessionStart", None),
    ("PreToolUse", Some("^Bash$")),
    ("PostToolUse", Some("^(Bash|apply_patch)$")),
    ("Stop", None),
    ("SubagentStop", None),
    ("PreCompact", None),
    ("SessionEnd", None),
];

const TRUST_STEP: &str = "Codex runs project hooks only after you trust them: open Codex in this repo, run /hooks, and trust the firsthand entries (trust follows the hook hash, so trust again after `firsthand init` changes them)";

/// First line of the managed block in `config.toml`.
const BLOCK_BEGIN: &str =
    "# firsthand:begin (managed by `firsthand init`; `firsthand uninstall` removes this block)";
/// Variant used when a line break had to be added before the block.
const BLOCK_BEGIN_NL: &str =
    "# firsthand:begin+nl (managed by `firsthand init`; `firsthand uninstall` removes this block)";
const BLOCK_END: &str = "# firsthand:end";

impl Codex {
    /// Adapter looking at `host`.
    pub fn new(host: HostEnv) -> Self {
        Codex { host }
    }

    fn codex_home(&self) -> Result<PathBuf> {
        match self.host.var("CODEX_HOME") {
            Some(d) => Ok(PathBuf::from(d)),
            None => Ok(util::home(&self.host)?.join(".codex")),
        }
    }

    fn dir(&self, repo: &RepoCtx, scope: HookScope) -> Result<PathBuf> {
        match scope {
            HookScope::Repo => Ok(repo.root.join(".codex")),
            HookScope::User => self.codex_home(),
        }
    }

    pub(crate) fn uninstall(&self, repo: &RepoCtx, scope: HookScope) -> Result<HookPlan> {
        let dir = self.dir(repo, scope)?;
        let mut edits: Vec<FileEdit> = json_uninstall(dir.join("hooks.json"), scope, None)?
            .into_iter()
            .collect();
        let toml = dir.join("config.toml");
        if let Some(before) = read_opt(&toml)? {
            let text = String::from_utf8_lossy(&before).into_owned();
            let after = block_remove(&text);
            if after != text {
                edits.push(FileEdit {
                    description: format!("remove the firsthand block from {}", toml.display()),
                    path: toml,
                    after: after.into_bytes(),
                    before: Some(before),
                    scope,
                });
            }
        }
        Ok(HookPlan {
            agent: AgentId::Codex,
            edits,
            manual_steps: Vec::new(),
        })
    }
}

/// Append a managed block, adding a line break first if the text lacks one.
pub(crate) fn block_insert(text: &str, body: &str) -> String {
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let body = body.replace('\n', nl);
    if text.is_empty() || text.ends_with('\n') {
        format!("{text}{BLOCK_BEGIN}{nl}{body}{BLOCK_END}{nl}")
    } else {
        format!("{text}{nl}{BLOCK_BEGIN_NL}{nl}{body}{BLOCK_END}{nl}")
    }
}

/// Remove the managed block exactly as [`block_insert`] added it.
pub(crate) fn block_remove(text: &str) -> String {
    let Some(start) = text.find("# firsthand:begin") else {
        return text.to_owned();
    };
    let Some(end_rel) = text[start..].find(BLOCK_END) else {
        return text.to_owned();
    };
    let mut end = start + end_rel + BLOCK_END.len();
    if text[end..].starts_with("\r\n") {
        end += 2;
    } else if text[end..].starts_with('\n') {
        end += 1;
    }
    let mut begin = start;
    if text[start..].starts_with(BLOCK_BEGIN_NL) {
        if text[..start].ends_with("\r\n") {
            begin -= 2;
        } else if text[..start].ends_with('\n') {
            begin -= 1;
        }
    }
    format!("{}{}", &text[..begin], &text[end..])
}

fn toml_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| format!("\"{s}\""))
}

impl AgentAdapter for Codex {
    fn id(&self) -> AgentId {
        AgentId::Codex
    }

    fn detect(&self, host: &HostEnv) -> Detection {
        let me = Codex::new(host.clone());
        let configs = me
            .codex_home()
            .map(|d| {
                vec![
                    d.join("config.toml"),
                    d.join("hooks.json"),
                    d.join("sessions"),
                ]
            })
            .unwrap_or_default();
        util::detect(host, "codex", configs)
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            witness_bash: Witness::Partial,
            witness_edits: Witness::Partial,
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
        let root = plain(&repo.root);
        let mut files = Vec::new();
        collect_rollouts(&self.codex_home()?.join("sessions"), 0, &mut files);
        let mut out: Vec<SessionRef> = files
            .into_iter()
            .filter_map(|path| {
                let (id, cwd) = rollout_meta(&path);
                let cwd = cwd.filter(|c| path_inside(c, &root))?;
                Some(SessionRef {
                    agent: AgentId::Codex,
                    id: id.unwrap_or_else(|| {
                        path.file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    }),
                    modified: mtime(&path),
                    path,
                    cwd: Some(cwd),
                })
            })
            .collect();
        out.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    fn ingest(&self, sref: &SessionRef, opts: &IngestOpts) -> Result<IngestedSession> {
        let src = util::read_source(&sref.path, opts.max_bytes)?;
        let mut acc = Acc::default();
        // The session meta line may fall outside a tail read; take it from the head.
        if let Some(first) = head_lines(&sref.path, 1, 1024 * 1024).first()
            && let Ok(v) = serde_json::from_str::<Value>(first)
        {
            meta(&mut acc, &v);
        }
        for line in lines(&src.bytes) {
            match serde_json::from_str::<Value>(&line) {
                Ok(v) if v.is_object() => record(&mut acc, &v),
                _ => acc.skipped += 1,
            }
        }
        Ok(acc.finish(AgentId::Codex, &sref.id, src.hash, opts))
    }

    fn hook_plan(&self, repo: &RepoCtx, scope: HookScope) -> Result<HookPlan> {
        let path = self.dir(repo, scope)?.join("hooks.json");
        let entries = hook_entries(repo, AgentId::Codex, &EVENTS);
        let desc = format!("add firsthand hooks to {}", path.display());
        let edit = json_edit(path, scope, desc, |t| {
            install_hooks(t, &entries, &json!({}))
        })?;
        let mut steps = vec![TRUST_STEP.to_owned()];
        if scope == HookScope::Repo {
            steps.push(
                "Codex loads project config only for trusted projects: trust this folder when Codex asks".into(),
            );
        }
        Ok(HookPlan {
            agent: AgentId::Codex,
            edits: edit.into_iter().collect(),
            manual_steps: steps,
        })
    }

    fn parse_hook_event(&self, raw: &[u8], argv_tail: &[String]) -> Result<HookEvent> {
        let v = hookparse::payload(raw, argv_tail)?;
        let mut ev = base_event(AgentId::Codex, &v, argv_tail);
        if let Some(name) = s(&v, "tool_name") {
            ev.tool = Some(codex_tool(name, &v, ev.kind));
        }
        Ok(ev)
    }

    fn mcp_install_plan(&self, repo: &RepoCtx, scope: HookScope) -> Result<ConfigPlan> {
        let path = self.dir(repo, scope)?.join("config.toml");
        let before = read_opt(&path)?;
        let text = match &before {
            Some(b) => String::from_utf8(b.clone()).map_err(|_| Error::Config {
                path: path.clone(),
                reason: "not valid UTF-8; fix the file by hand and run again".into(),
            })?,
            None => String::new(),
        };
        let argv = plan::mcp_argv(repo);
        let args: Vec<String> = argv[1..].iter().map(|a| toml_str(a)).collect();
        let body = format!(
            "[mcp_servers.firsthand]\ncommand = {}\nargs = [{}]\n",
            toml_str(&argv[0]),
            args.join(", ")
        );
        let current = block_remove(&text);
        let mut steps = Vec::new();
        let edits = if current.contains("[mcp_servers.firsthand]") {
            steps.push(format!(
                "{} already defines [mcp_servers.firsthand] outside the firsthand block; left unchanged",
                path.display()
            ));
            Vec::new()
        } else {
            let after = if text
                .contains(&body.replace('\n', if text.contains("\r\n") { "\r\n" } else { "\n" }))
            {
                text.clone()
            } else {
                block_insert(&current, &body)
            };
            if after == text {
                Vec::new()
            } else {
                vec![FileEdit {
                    description: format!("add the firsthand MCP server to {}", path.display()),
                    path,
                    before,
                    after: after.into_bytes(),
                    scope,
                }]
            }
        };
        if scope == HookScope::Repo {
            steps.push(
                "Codex reads <repo>/.codex/config.toml only for trusted projects: trust this folder when Codex asks".into(),
            );
        }
        Ok(HookPlan {
            agent: AgentId::Codex,
            edits,
            manual_steps: steps,
        })
    }

    fn launch_spec(&self, brief: &BriefRef, repo: &RepoCtx) -> Result<LaunchSpec> {
        Ok(LaunchSpec {
            argv: vec!["codex".into(), brief.bootstrap.clone()],
            env: Vec::new(),
            cwd: repo.root.clone(),
            verified: false,
        })
    }
}

fn collect_rollouts(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && depth < 4 {
            collect_rollouts(&p, depth + 1, out);
        } else if p.extension().is_some_and(|x| x == "jsonl")
            && e.file_name().to_string_lossy().starts_with("rollout-")
        {
            out.push(p);
        }
    }
}

/// Session id and cwd from the first lines of a rollout.
fn rollout_meta(path: &Path) -> (Option<String>, Option<String>) {
    let mut acc = Acc::default();
    for l in head_lines(path, 20, 1024 * 1024) {
        if let Ok(v) = serde_json::from_str::<Value>(&l) {
            meta(&mut acc, &v);
            if acc.cwd.is_some() {
                break;
            }
        }
    }
    (acc.session_id, acc.cwd)
}

fn meta(acc: &mut Acc, v: &Value) {
    let p = v.get("payload").unwrap_or(v);
    match s(v, "type") {
        Some("session_meta") => {
            acc.session_id = s(p, "id").map(str::to_owned).or(acc.session_id.take());
            acc.cwd = s(p, "cwd").map(str::to_owned).or(acc.cwd.take());
            if let Some(b) = p.get("git").and_then(|g| s(g, "branch")) {
                acc.git_branch = Some(b.to_owned());
            }
        }
        Some("turn_context") if acc.cwd.is_none() => acc.cwd = s(p, "cwd").map(str::to_owned),
        // Older rollouts: a bare header object first.
        None if p.get("instructions").is_some() || p.get("id").is_some() => {
            if acc.session_id.is_none() {
                acc.session_id = s(p, "id").map(str::to_owned);
            }
            if acc.cwd.is_none() {
                acc.cwd = s(p, "cwd").map(str::to_owned);
            }
        }
        _ => {}
    }
}

/// Fold one rollout line into the accumulator.
fn record(acc: &mut Acc, v: &Value) {
    let t = ts(v.get("timestamp"));
    let (kind, item) = match (s(v, "type"), v.get("payload")) {
        (Some(k), Some(p)) => (k, p),
        _ => ("response_item", v),
    };
    match kind {
        "session_meta" | "turn_context" => meta(acc, v),
        "compacted" => {
            if let Some(m) = s(item, "message") {
                acc.message(Role::System, m, t, true);
            }
        }
        "response_item" => item_record(acc, item, t),
        // event_msg lines repeat response items for the UI.
        _ => {}
    }
}

fn item_record(acc: &mut Acc, item: &Value, t: Option<fh_core::Ts>) {
    acc.saw(t.as_ref());
    match s(item, "type").unwrap_or("") {
        "message" => {
            let text = item.get("content").map(text_of).unwrap_or_default();
            let role = match s(item, "role") {
                Some("assistant") => Role::Assistant,
                Some("user") if !is_injected_context(&text) => Role::User,
                _ => Role::System,
            };
            acc.message(role, &text, t, false);
        }
        "function_call" | "custom_tool_call" | "local_shell_call" => {
            let name = s(item, "name").unwrap_or(if s(item, "type") == Some("local_shell_call") {
                "local_shell"
            } else {
                "tool"
            });
            let input = match item.get("arguments").or_else(|| item.get("input")) {
                Some(Value::String(a)) => {
                    serde_json::from_str(a).unwrap_or(Value::String(a.clone()))
                }
                Some(other) => other.clone(),
                None => item.get("action").cloned().unwrap_or(Value::Null),
            };
            let summary = match hookparse::find_string(&input, "*** Begin Patch") {
                Some(p) => patch_files(p).0.join(", "),
                None => summarize_input(&input),
            };
            acc.tool_use(
                s(item, "call_id").or_else(|| s(item, "id")),
                name,
                summary,
                t,
            );
        }
        "function_call_output" | "custom_tool_call_output" => {
            let raw = item.get("output").cloned().unwrap_or(Value::Null);
            let parsed = match &raw {
                Value::String(o) => serde_json::from_str::<Value>(o).unwrap_or(raw.clone()),
                other => other.clone(),
            };
            let code = parsed
                .get("metadata")
                .and_then(|m| m.get("exit_code"))
                .and_then(Value::as_i64);
            let failed = parsed.get("success") == Some(&Value::Bool(false));
            let text = match &parsed {
                Value::Object(o) => o
                    .get("output")
                    .or_else(|| o.get("content"))
                    .map(text_of)
                    .unwrap_or_default(),
                other => text_of(other),
            };
            let code = code.or_else(|| hookparse::exit_code_in(&text).map(i64::from));
            acc.tool_result(
                s(item, "call_id"),
                &text,
                failed || code.is_some_and(|c| c != 0),
            );
        }
        // reasoning items are hidden reasoning: never ingested.
        _ => {}
    }
}

/// Context Codex injects as user messages (environment, AGENTS.md instructions).
fn is_injected_context(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("<environment_context>")
        || t.starts_with("<user_instructions>")
        || t.starts_with("# AGENTS.md instructions")
}

fn codex_tool(name: &str, v: &Value, kind: HookEventKind) -> fh_core::ToolCall {
    let input = v.get("tool_input").unwrap_or(&Value::Null);
    let post = kind == HookEventKind::PostToolUse;
    let resp = v.get("tool_response").unwrap_or(&Value::Null);
    if let Some(p) = hookparse::find_string(input, "*** Begin Patch") {
        let mut t = tool(name, ToolClass::FileEdit);
        (t.file_paths, t.file_op) = patch_files(p);
        if post {
            let (code, _, _) = shell_result(resp);
            t.is_error = Some(
                code.is_some_and(|c| c != 0) || resp.get("success") == Some(&Value::Bool(false)),
            );
        }
        return t;
    }
    match name {
        "Bash" | "shell" | "local_shell" | "exec_command" | "unified_exec" => {
            let mut t = tool(name, ToolClass::Shell);
            (t.command, t.argv) = hookparse::shell_command(input);
            if post {
                let (code, out, err) = shell_result(resp);
                t.exit_code = code;
                t.stdout = out;
                t.stderr = err;
                t.is_error = code.map(|c| c != 0);
            }
            t
        }
        _ => other_tool(name, input),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_block_round_trip() {
        for orig in ["", "model = \"x\"\n", "model = \"x\"", "a = 1\r\n"] {
            let body =
                "[mcp_servers.firsthand]\ncommand = \"firsthand\"\nargs = [\"mcp\", \"serve\"]\n";
            let with = block_insert(orig, body);
            assert!(with.contains("[mcp_servers.firsthand]"));
            assert!(toml::from_str::<toml::Table>(&with).is_ok(), "{with}");
            assert_eq!(block_remove(&with), orig);
        }
    }
}

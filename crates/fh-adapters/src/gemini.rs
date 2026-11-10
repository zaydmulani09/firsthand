//! Gemini CLI: chats under `~/.gemini/tmp/<project>/chats/`, hooks and MCP in
//! `.gemini/settings.json`.

use crate::hookparse::{self, base_event, other_tool, tool};
use crate::ingest::Acc;
use crate::plan::{self, hook_entries, install_hooks, json_edit, json_uninstall};
use crate::util::{self, mtime, norm_path, path_inside, plain, s, summarize_input, text_of, ts};
use crate::{
    AgentAdapter, BriefRef, Capabilities, ConfigPlan, Detection, HookPlan, HookScope, HostEnv,
    IngestOpts, Inject, LaunchSpec, RepoCtx, SessionRef, Witness,
};
use fh_core::{
    AgentId, Error, FileOp, HookEvent, HookEventKind, IngestedSession, Result, Role, ToolClass,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// Gemini CLI adapter.
#[derive(Debug, Clone)]
pub struct GeminiCli {
    host: HostEnv,
}

const EVENTS: [(&str, Option<&str>); 6] = [
    ("SessionStart", None),
    ("BeforeTool", Some("run_shell_command")),
    ("AfterTool", Some("run_shell_command|write_file|replace")),
    ("AfterAgent", None),
    ("PreCompress", None),
    ("SessionEnd", None),
];

impl GeminiCli {
    /// Adapter looking at `host`.
    pub fn new(host: HostEnv) -> Self {
        GeminiCli { host }
    }

    fn gemini_dir(&self) -> Result<PathBuf> {
        Ok(util::home(&self.host)?.join(".gemini"))
    }

    fn settings(&self, repo: &RepoCtx, scope: HookScope) -> Result<PathBuf> {
        Ok(match scope {
            HookScope::Repo => repo.root.join(".gemini").join("settings.json"),
            HookScope::User => self.gemini_dir()?.join("settings.json"),
        })
    }

    pub(crate) fn uninstall(&self, repo: &RepoCtx, scope: HookScope) -> Result<HookPlan> {
        let edit = json_uninstall(self.settings(repo, scope)?, scope, Some("mcpServers"))?;
        Ok(HookPlan {
            agent: AgentId::GeminiCli,
            edits: edit.into_iter().collect(),
            manual_steps: Vec::new(),
        })
    }

    /// Project directories under `~/.gemini/tmp` that belong to `root`.
    fn project_dirs(&self, root: &str) -> Result<Vec<(PathBuf, String)>> {
        let tmp = self.gemini_dir()?.join("tmp");
        let mut names: Vec<(String, String)> = Vec::new();
        // Hash of the project root, as Gemini CLI computes it.
        let variants = [root.to_owned(), norm_path(root), root.replace('/', "\\")];
        for v in variants {
            names.push((hex_sha256(&v), root.to_owned()));
        }
        // Newer layouts map paths to readable slugs.
        if let Ok(b) = std::fs::read(self.gemini_dir()?.join("projects.json"))
            && let Ok(v) = serde_json::from_slice::<Value>(&b)
        {
            let map = v.get("projects").unwrap_or(&v);
            if let Some(o) = map.as_object() {
                for (path, slug) in o {
                    if let Some(slug) = slug.as_str()
                        && path_inside(path, root)
                    {
                        names.push((slug.to_owned(), path.clone()));
                    }
                }
            }
        }
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(&tmp) else {
            return Ok(out);
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let marker = std::fs::read_to_string(e.path().join(".project_root")).ok();
            let hit = names
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, p)| p.clone())
                .or_else(|| {
                    marker
                        .map(|m| m.trim().to_owned())
                        .filter(|m| path_inside(m, root))
                });
            if let Some(cwd) = hit {
                out.push((e.path(), cwd));
            }
        }
        Ok(out)
    }
}

fn hex_sha256(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl AgentAdapter for GeminiCli {
    fn id(&self) -> AgentId {
        AgentId::GeminiCli
    }

    fn detect(&self, host: &HostEnv) -> Detection {
        let configs = GeminiCli::new(host.clone())
            .gemini_dir()
            .map(|d| vec![d.join("settings.json"), d.join("tmp")])
            .unwrap_or_default();
        util::detect(host, "gemini", configs)
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            witness_bash: Witness::Partial,
            witness_edits: Witness::Partial,
            pre_tool_events: true,
            session_start_inject: Inject::Stdout,
            stop_event: true,
            compact_event: true,
            subagent_events: false,
            ingest: true,
            mcp: true,
        }
    }

    fn discover_sessions(&self, repo: &RepoCtx) -> Result<Vec<SessionRef>> {
        let root = plain(&repo.root);
        let mut out = Vec::new();
        for (dir, cwd) in self.project_dirs(&root)? {
            let Ok(rd) = std::fs::read_dir(dir.join("chats")) else {
                continue;
            };
            for e in rd.flatten() {
                let path = e.path();
                if !path
                    .extension()
                    .is_some_and(|x| x == "json" || x == "jsonl")
                {
                    continue;
                }
                out.push(SessionRef {
                    agent: AgentId::GeminiCli,
                    id: path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    modified: mtime(&path),
                    path,
                    cwd: Some(cwd.clone()),
                });
            }
        }
        out.sort_by(|a, b| b.modified.cmp(&a.modified).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    fn ingest(&self, sref: &SessionRef, opts: &IngestOpts) -> Result<IngestedSession> {
        let src = util::read_source(&sref.path, opts.max_bytes)?;
        let text = String::from_utf8_lossy(&src.bytes);
        let mut acc = Acc::default();
        acc.cwd = sref.cwd.clone();
        let mut stream = serde_json::Deserializer::from_str(&text).into_iter::<Value>();
        let mut values = Vec::new();
        loop {
            match stream.next() {
                Some(Ok(v)) => values.push(v),
                Some(Err(_)) => {
                    // Malformed tail: count the remaining non-empty lines as skipped.
                    let rest = &text[stream.byte_offset()..];
                    acc.skipped +=
                        rest.lines().filter(|l| !l.trim().is_empty()).count().max(1) as u32;
                    break;
                }
                None => break,
            }
        }
        if values.is_empty() {
            return Err(Error::Invalid {
                what: "gemini session",
                value: sref.path.display().to_string(),
                reason: "no JSON records found".into(),
            });
        }
        for v in &values {
            if let Some(msgs) = v.get("messages").and_then(Value::as_array) {
                header(&mut acc, v);
                for m in msgs {
                    message(&mut acc, m);
                }
            } else if v.get("type").is_some()
                && (v.get("content").is_some() || v.get("toolCalls").is_some())
            {
                message(&mut acc, v);
            } else if v.get("sessionId").is_some() {
                header(&mut acc, v);
            } else {
                acc.skipped += 1;
            }
        }
        Ok(acc.finish(AgentId::GeminiCli, &sref.id, src.hash, opts))
    }

    fn hook_plan(&self, repo: &RepoCtx, scope: HookScope) -> Result<HookPlan> {
        let path = self.settings(repo, scope)?;
        let entries = hook_entries(repo, AgentId::GeminiCli, &EVENTS);
        let desc = format!("add firsthand hooks to {}", path.display());
        let extra = json!({"name": "firsthand", "timeout": 30000});
        let edit = json_edit(path, scope, desc, |t| install_hooks(t, &entries, &extra))?;
        let mut steps = Vec::new();
        if scope == HookScope::Repo {
            steps.push("if .gemini/settings.json is tracked by git, this edit changes a tracked file; use --scope user instead to keep the repo clean".into());
        }
        Ok(HookPlan {
            agent: AgentId::GeminiCli,
            edits: edit.into_iter().collect(),
            manual_steps: steps,
        })
    }

    fn parse_hook_event(&self, raw: &[u8], argv_tail: &[String]) -> Result<HookEvent> {
        let v = hookparse::payload(raw, argv_tail)?;
        let mut ev = base_event(AgentId::GeminiCli, &v, argv_tail);
        if let Some(name) = s(&v, "tool_name") {
            ev.tool = Some(gemini_tool(name, &v, ev.kind));
        }
        Ok(ev)
    }

    fn mcp_install_plan(&self, repo: &RepoCtx, scope: HookScope) -> Result<ConfigPlan> {
        let path = self.settings(repo, scope)?;
        let argv = plan::mcp_argv(repo);
        let entry = json!({"command": argv[0], "args": argv[1..]});
        let desc = format!("add the firsthand MCP server to {}", path.display());
        let edit = json_edit(path, scope, desc, |t| {
            crate::jsonedit::insert_member(t, &["mcpServers"], "firsthand", &entry)
        })?;
        Ok(HookPlan {
            agent: AgentId::GeminiCli,
            edits: edit.into_iter().collect(),
            manual_steps: vec![
                "hooks and MCP share .gemini/settings.json: apply the hook plan before computing this one".into(),
            ],
        })
    }

    fn launch_spec(&self, brief: &BriefRef, repo: &RepoCtx) -> Result<LaunchSpec> {
        Ok(LaunchSpec {
            argv: vec!["gemini".into(), "-i".into(), brief.bootstrap.clone()],
            env: Vec::new(),
            cwd: repo.root.clone(),
            verified: false,
        })
    }
}

fn header(acc: &mut Acc, v: &Value) {
    if acc.session_id.is_none() {
        acc.session_id = s(v, "sessionId").map(str::to_owned);
    }
    acc.saw(ts(v.get("startTime")).as_ref());
    acc.saw(ts(v.get("lastUpdated")).as_ref());
}

fn message(acc: &mut Acc, m: &Value) {
    let t = ts(m.get("timestamp"));
    let role = match s(m, "type") {
        Some("user") => Role::User,
        Some("gemini" | "model" | "assistant") => Role::Assistant,
        _ => Role::System,
    };
    // `thoughts` hold reasoning summaries: never ingested.
    let text = m.get("content").map(text_of).unwrap_or_default();
    acc.message(role, &text, t.clone(), false);
    for call in m
        .get("toolCalls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = s(call, "name").unwrap_or("tool");
        let args = call.get("args").unwrap_or(&Value::Null);
        let id = s(call, "id");
        acc.tool_use(
            id,
            name,
            summarize_input(args),
            ts(call.get("timestamp")).or(t.clone()),
        );
        let out = match call.get("resultDisplay") {
            Some(Value::String(d)) => d.clone(),
            _ => call
                .get("result")
                .and_then(|r| {
                    hookparse::find_key(r, "output").or_else(|| hookparse::find_key(r, "error"))
                })
                .map(str::to_owned)
                .unwrap_or_default(),
        };
        acc.tool_result(id, &out, s(call, "status") == Some("error"));
    }
}

/// Gemini's shell tool reports `Output:` and `Exit Code:` lines in `llmContent`.
fn shell_text(text: &str) -> (Option<i32>, Option<String>, Option<String>) {
    let code = hookparse::exit_code_in(text);
    let section = |name: &str| -> Option<String> {
        let start = text.find(&format!("{name}: "))? + name.len() + 2;
        let rest = &text[start..];
        let end = [
            "\nError: ",
            "\nExit Code: ",
            "\nSignal: ",
            "\nBackground PIDs: ",
            "\nProcess Group PGID: ",
        ]
        .iter()
        .filter_map(|m| rest.find(m))
        .min()
        .unwrap_or(rest.len());
        Some(rest[..end].to_owned()).filter(|s| s != "(empty)" && s != "(none)")
    };
    (code, section("Output"), section("Error"))
}

fn gemini_tool(name: &str, v: &Value, kind: HookEventKind) -> fh_core::ToolCall {
    let input = v.get("tool_input").unwrap_or(&Value::Null);
    let resp = v.get("tool_response").unwrap_or(&Value::Null);
    let post = kind == HookEventKind::PostToolUse;
    let errored = resp.get("error").is_some_and(|e| !e.is_null());
    match name {
        "run_shell_command" => {
            let mut t = tool(name, ToolClass::Shell);
            (t.command, t.argv) = hookparse::shell_command(input);
            if post {
                let text = resp.get("llmContent").map(text_of).unwrap_or_default();
                let (code, out, err) = shell_text(&text);
                t.exit_code = code;
                t.stdout = out;
                t.stderr = err;
                t.is_error = Some(errored || code.is_some_and(|c| c != 0));
            }
            t
        }
        "write_file" | "replace" | "edit" => {
            let mut t = tool(name, ToolClass::FileEdit);
            t.file_paths = ["file_path", "absolute_path", "path"]
                .iter()
                .find_map(|k| s(input, k))
                .map(str::to_owned)
                .into_iter()
                .collect();
            t.file_op = Some(if name == "write_file" {
                FileOp::Write
            } else {
                FileOp::Edit
            });
            if post {
                t.is_error = Some(errored);
            }
            t
        }
        _ => other_tool(name, input),
    }
}

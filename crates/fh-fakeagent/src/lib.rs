//! Fake agent for tests: writes session logs in each agent's on-disk format under a
//! fake home directory, and emits harness-shaped hook invocations by spawning a
//! `firsthand` binary with the agent's JSON on stdin (or as the last argument for
//! Codex legacy `notify`). All content is synthetic.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use fh_adapters::encode_project_dir;
use fh_core::{AgentId, HookEventKind, Ts};
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// Base instant of every synthetic timestamp (matches `fh-testkit`).
pub const BASE_TIME: &str = "2026-11-09T14:02:11.123Z";

fn at(step: i64) -> String {
    let base = Ts::parse(BASE_TIME).map(|t| t.unix_ms()).unwrap_or(0);
    Ts::from_unix_ms(base + step * 1000).as_str().to_owned()
}

fn at_ms(step: i64) -> i64 {
    Ts::parse(BASE_TIME).map(|t| t.unix_ms()).unwrap_or(0) + step * 1000
}

/// Kind of a fake tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    /// Shell command.
    Shell,
    /// Whole-file write.
    Write,
    /// In-place edit.
    Edit,
}

/// A fake tool call and its result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSpec {
    /// Kind.
    pub kind: ToolKind,
    /// Shell command text (shell tools).
    pub command: String,
    /// Exit code (shell tools).
    pub exit_code: i32,
    /// stdout (shell tools).
    pub stdout: String,
    /// stderr (shell tools).
    pub stderr: String,
    /// File path relative to the session cwd (edit tools).
    pub file: String,
    /// Tool use id.
    pub tool_use_id: String,
}

impl ToolSpec {
    /// A shell call.
    pub fn shell(id: &str, command: &str, exit_code: i32, stdout: &str) -> Self {
        ToolSpec {
            kind: ToolKind::Shell,
            command: command.into(),
            exit_code,
            stdout: stdout.into(),
            stderr: String::new(),
            file: String::new(),
            tool_use_id: id.into(),
        }
    }

    /// An edit-tool call.
    pub fn edit(id: &str, kind: ToolKind, file: &str) -> Self {
        ToolSpec {
            kind,
            command: String::new(),
            exit_code: 0,
            stdout: String::new(),
            stderr: String::new(),
            file: file.into(),
            tool_use_id: id.into(),
        }
    }
}

/// One hook invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookSpec {
    /// Normalized event.
    pub kind: HookEventKind,
    /// Session id.
    pub session_id: String,
    /// Working directory.
    pub cwd: PathBuf,
    /// SessionStart source.
    pub source: Option<String>,
    /// Tool call, for tool events.
    pub tool: Option<ToolSpec>,
}

/// The harness's own event name for a normalized event, or `None` when the agent has
/// no such event. A failed shell call under Claude Code is `PostToolUseFailure`.
pub fn event_name(agent: AgentId, spec: &HookSpec) -> Option<&'static str> {
    use HookEventKind as K;
    let failed = spec
        .tool
        .as_ref()
        .is_some_and(|t| t.kind == ToolKind::Shell && t.exit_code != 0);
    Some(match (agent, spec.kind) {
        (AgentId::ClaudeCode, K::PostToolUse) if failed => "PostToolUseFailure",
        (AgentId::ClaudeCode | AgentId::Codex, k) => match k {
            K::SessionStart => "SessionStart",
            K::PreToolUse => "PreToolUse",
            K::PostToolUse => "PostToolUse",
            K::Stop => "Stop",
            K::SubagentStop => "SubagentStop",
            K::PreCompact => "PreCompact",
            K::SessionEnd => "SessionEnd",
            K::UserPromptSubmit => "UserPromptSubmit",
            K::Other => return None,
        },
        (AgentId::GeminiCli, k) => match k {
            K::SessionStart => "SessionStart",
            K::PreToolUse => "BeforeTool",
            K::PostToolUse => "AfterTool",
            K::Stop => "AfterAgent",
            K::PreCompact => "PreCompress",
            K::SessionEnd => "SessionEnd",
            K::UserPromptSubmit => "BeforeAgent",
            _ => return None,
        },
        (AgentId::Opencode, k) => match k {
            K::SessionStart => "session.created",
            K::PreToolUse => "tool.execute.before",
            K::PostToolUse => "tool.execute.after",
            K::Stop => "session.idle",
            K::PreCompact => "session.compacted",
            _ => return None,
        },
        (AgentId::Generic, _) => return None,
    })
}

fn abs(cwd: &Path, rel: &str) -> String {
    cwd.join(rel).to_string_lossy().into_owned()
}

/// Harness-shaped JSON for one hook invocation.
pub fn hook_payload(agent: AgentId, spec: &HookSpec) -> Value {
    let name = event_name(agent, spec).unwrap_or("Other");
    let cwd = spec.cwd.to_string_lossy().into_owned();
    let post = spec.kind == HookEventKind::PostToolUse;
    let mut v = match agent {
        AgentId::Opencode => {
            json!({"hook_event_name": name, "cwd": cwd, "session_id": spec.session_id})
        }
        _ => json!({
            "session_id": spec.session_id,
            "transcript_path": abs(&spec.cwd, ".fake/transcript.jsonl"),
            "cwd": cwd,
            "hook_event_name": name,
        }),
    };
    let o = v.as_object_mut().unwrap();
    if let Some(src) = &spec.source {
        o.insert("source".into(), json!(src));
    }
    if agent == AgentId::GeminiCli {
        o.insert("timestamp".into(), json!(at(0)));
    }
    let Some(t) = &spec.tool else { return v };
    let (tool_name, input) = match (agent, t.kind) {
        (AgentId::GeminiCli, ToolKind::Shell) => {
            ("run_shell_command", json!({"command": t.command}))
        }
        (AgentId::GeminiCli, ToolKind::Write) => (
            "write_file",
            json!({"file_path": abs(&spec.cwd, &t.file), "content": "synthetic\n"}),
        ),
        (AgentId::GeminiCli, ToolKind::Edit) => (
            "replace",
            json!({"file_path": abs(&spec.cwd, &t.file), "old_string": "a", "new_string": "b"}),
        ),
        (AgentId::Opencode, ToolKind::Shell) => (
            "bash",
            json!({"command": t.command, "description": "synthetic"}),
        ),
        (AgentId::Opencode, ToolKind::Write) => (
            "write",
            json!({"filePath": abs(&spec.cwd, &t.file), "content": "synthetic\n"}),
        ),
        (AgentId::Opencode, ToolKind::Edit) => (
            "edit",
            json!({"filePath": abs(&spec.cwd, &t.file), "oldString": "a", "newString": "b"}),
        ),
        (AgentId::Codex, ToolKind::Shell) => ("Bash", json!({"command": t.command})),
        (AgentId::Codex, _) => (
            "apply_patch",
            json!({"command": format!("*** Begin Patch\n*** {} File: {}\n+synthetic\n*** End Patch\n", if t.kind == ToolKind::Write { "Add" } else { "Update" }, t.file)}),
        ),
        (_, ToolKind::Shell) => (
            "Bash",
            json!({"command": t.command, "description": "synthetic"}),
        ),
        (_, ToolKind::Write) => (
            "Write",
            json!({"file_path": abs(&spec.cwd, &t.file), "content": "synthetic\n"}),
        ),
        (_, ToolKind::Edit) => (
            "Edit",
            json!({"file_path": abs(&spec.cwd, &t.file), "old_string": "a", "new_string": "b"}),
        ),
    };
    o.insert("tool_name".into(), json!(tool_name));
    o.insert("tool_input".into(), input);
    o.insert("tool_use_id".into(), json!(t.tool_use_id));
    if !post {
        return v;
    }
    let shell = t.kind == ToolKind::Shell;
    match agent {
        AgentId::ClaudeCode if shell && t.exit_code != 0 => {
            o.insert(
                "error".into(),
                json!(format!(
                    "Exit code {}\n{}{}",
                    t.exit_code, t.stdout, t.stderr
                )),
            );
            o.insert("is_interrupt".into(), json!(false));
        }
        AgentId::ClaudeCode if shell => {
            o.insert("tool_response".into(), json!({"stdout": t.stdout, "stderr": t.stderr, "interrupted": false, "isImage": false}));
        }
        AgentId::ClaudeCode => {
            o.insert(
                "tool_response".into(),
                json!({"filePath": abs(&spec.cwd, &t.file), "type": "update"}),
            );
        }
        AgentId::Codex => {
            o.insert(
                "tool_response".into(),
                json!(format!(
                    "Exit code: {}\nWall time: 0.1 seconds\nOutput:\n{}{}",
                    t.exit_code, t.stdout, t.stderr
                )),
            );
        }
        AgentId::GeminiCli if shell => {
            let err = if t.stderr.is_empty() {
                "(none)"
            } else {
                &t.stderr
            };
            o.insert(
                "tool_response".into(),
                json!({
                    "llmContent": format!("Command: {}\nDirectory: (root)\nOutput: {}\nError: {err}\nExit Code: {}\nSignal: (none)\nBackground PIDs: (none)\nProcess Group PGID: 4242", t.command, t.stdout, t.exit_code),
                    "returnDisplay": t.stdout,
                }),
            );
        }
        AgentId::GeminiCli => {
            o.insert(
                "tool_response".into(),
                json!({"llmContent": "Successfully wrote file", "returnDisplay": "ok"}),
            );
        }
        AgentId::Opencode => {
            o.insert("tool_output".into(), json!(t.stdout));
            o.insert(
                "tool_metadata".into(),
                json!({"exit": t.exit_code, "output": t.stdout}),
            );
        }
        AgentId::Generic => {}
    }
    v
}

/// Codex legacy `notify` payload (passed as the last argument).
pub fn notify_payload(session_id: &str, cwd: &Path) -> Value {
    json!({
        "type": "agent-turn-complete",
        "thread-id": session_id,
        "turn-id": "turn-1",
        "cwd": cwd.to_string_lossy(),
        "input-messages": ["synthetic request"],
        "last-assistant-message": "synthetic reply",
    })
}

/// Spawn `<firsthand> hook <agent> <EventName>` with the payload on stdin (or, with
/// `notify`, as the last argument and empty stdin, the Codex legacy form).
pub fn invoke(
    firsthand: &Path,
    agent: AgentId,
    spec: &HookSpec,
    notify: bool,
) -> std::io::Result<Output> {
    let (name, stdin, extra) = if notify {
        (
            "Stop",
            String::new(),
            Some(notify_payload(&spec.session_id, &spec.cwd).to_string()),
        )
    } else {
        let name = event_name(agent, spec).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!("{} has no {:?} event", agent.as_str(), spec.kind),
            )
        })?;
        (name, hook_payload(agent, spec).to_string(), None)
    };
    let mut cmd = Command::new(firsthand);
    cmd.args(["hook", agent.as_str(), name])
        .args(extra)
        .current_dir(&spec.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    if let Some(mut si) = child.stdin.take() {
        si.write_all(stdin.as_bytes())?;
    }
    child.wait_with_output()
}

/// A realistic session: SessionStart, `pairs` Pre/Post tool pairs cycling through a
/// failing test run, an edit, a passing test run and a plain command, then Stop.
pub fn script(session_id: &str, cwd: &Path, pairs: usize) -> Vec<HookSpec> {
    let mk = |kind, tool: Option<ToolSpec>| HookSpec {
        kind,
        session_id: session_id.into(),
        cwd: cwd.to_path_buf(),
        source: (kind == HookEventKind::SessionStart).then(|| "startup".into()),
        tool,
    };
    let mut out = vec![mk(HookEventKind::SessionStart, None)];
    for i in 0..pairs {
        let id = format!("toolu_fake_{i:04}");
        let t = match i % 4 {
            0 => ToolSpec::shell(
                &id,
                "cargo test",
                101,
                "test result: FAILED. 3 passed; 1 failed; 0 ignored\n",
            ),
            1 => ToolSpec::edit(&id, ToolKind::Edit, "src/lib.rs"),
            2 => ToolSpec::shell(
                &id,
                "cargo test",
                0,
                "test result: ok. 4 passed; 0 failed; 0 ignored\n",
            ),
            _ => ToolSpec::shell(&id, "ls", 0, "Cargo.toml\nsrc\n"),
        };
        out.push(mk(HookEventKind::PreToolUse, Some(t.clone())));
        out.push(mk(HookEventKind::PostToolUse, Some(t)));
    }
    out.push(mk(HookEventKind::Stop, None));
    out
}

/// One user turn of a fake session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    /// User text.
    pub user: String,
    /// Tool calls the assistant makes.
    pub tools: Vec<ToolSpec>,
    /// Assistant reply after the tools.
    pub assistant: String,
}

/// A fake session log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSpec {
    /// Session id.
    pub session_id: String,
    /// Working directory.
    pub cwd: PathBuf,
    /// Turns.
    pub turns: Vec<Turn>,
}

/// Hidden reasoning written into every log; ingestion must never return it.
pub const HIDDEN_REASONING: &str = "synthetic hidden reasoning that must not be ingested";

impl SessionSpec {
    /// A default two-turn session in `cwd`.
    pub fn sample(session_id: &str, cwd: &Path) -> Self {
        SessionSpec {
            session_id: session_id.into(),
            cwd: cwd.to_path_buf(),
            turns: vec![
                Turn {
                    user: "Add a parser for the config file.".into(),
                    tools: vec![
                        ToolSpec::edit("call_1", ToolKind::Write, "src/config.rs"),
                        ToolSpec::shell(
                            "call_2",
                            "cargo test",
                            0,
                            "test result: ok. 5 passed; 0 failed; 0 ignored\n",
                        ),
                    ],
                    assistant: "Added src/config.rs and the tests pass.".into(),
                },
                Turn {
                    user: "Now wire it into main.".into(),
                    tools: vec![ToolSpec::shell(
                        "call_3",
                        "cargo build",
                        0,
                        "Finished dev profile\n",
                    )],
                    assistant: "Wired the parser into main. Build is clean.".into(),
                },
            ],
        }
    }
}

/// Write a session log for `agent` under the fake `home`, laid out exactly where the
/// adapter looks. Returns the file written.
pub fn write_session(agent: AgentId, home: &Path, spec: &SessionSpec) -> std::io::Result<PathBuf> {
    match agent {
        AgentId::ClaudeCode => write_claude(home, spec),
        AgentId::Codex => write_codex(home, spec),
        AgentId::GeminiCli => write_gemini(home, spec),
        AgentId::Opencode => write_opencode(home, spec),
        AgentId::Generic => Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "generic agents have no session log",
        )),
    }
}

fn put(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, bytes)
}

fn jsonl(lines: &[Value]) -> Vec<u8> {
    let mut out = String::new();
    for l in lines {
        out.push_str(&l.to_string());
        out.push('\n');
    }
    out.into_bytes()
}

fn write_claude(home: &Path, spec: &SessionSpec) -> std::io::Result<PathBuf> {
    let cwd = spec.cwd.to_string_lossy().into_owned();
    let dir = home
        .join(".claude")
        .join("projects")
        .join(encode_project_dir(&cwd));
    let path = dir.join(format!("{}.jsonl", spec.session_id));
    let mut step = 0;
    let mut lines = Vec::new();
    let base = |kind: &str, message: Value, step: i64| {
        json!({"type": kind, "sessionId": spec.session_id, "cwd": cwd, "gitBranch": "main",
               "timestamp": at(step), "uuid": format!("u-{step}"), "isSidechain": false, "message": message})
    };
    for t in &spec.turns {
        step += 1;
        lines.push(base(
            "user",
            json!({"role": "user", "content": t.user}),
            step,
        ));
        for tool in &t.tools {
            step += 1;
            let (name, input) = match tool.kind {
                ToolKind::Shell => ("Bash", json!({"command": tool.command})),
                ToolKind::Write => (
                    "Write",
                    json!({"file_path": abs(&spec.cwd, &tool.file), "content": "synthetic\n"}),
                ),
                ToolKind::Edit => (
                    "Edit",
                    json!({"file_path": abs(&spec.cwd, &tool.file), "old_string": "a", "new_string": "b"}),
                ),
            };
            lines.push(base(
                "assistant",
                json!({"role": "assistant", "content": [
                    {"type": "thinking", "thinking": HIDDEN_REASONING, "signature": "sig"},
                    {"type": "tool_use", "id": tool.tool_use_id, "name": name, "input": input}
                ]}),
                step,
            ));
            step += 1;
            let out = if tool.kind == ToolKind::Shell {
                format!("{}{}", tool.stdout, tool.stderr)
            } else {
                "ok".into()
            };
            lines.push(base(
                "user",
                json!({"role": "user", "content": [
                    {"type": "tool_result", "tool_use_id": tool.tool_use_id, "content": out, "is_error": tool.exit_code != 0}
                ]}),
                step,
            ));
        }
        step += 1;
        lines.push(base(
            "assistant",
            json!({"role": "assistant", "content": [{"type": "text", "text": t.assistant}]}),
            step,
        ));
    }
    put(&path, &jsonl(&lines))?;
    Ok(path)
}

fn write_codex(home: &Path, spec: &SessionSpec) -> std::io::Result<PathBuf> {
    let path = home.join(".codex/sessions/2026/11/09").join(format!(
        "rollout-2026-11-09T14-02-11-{}.jsonl",
        spec.session_id
    ));
    let mut lines = vec![
        json!({"timestamp": at(0), "type": "session_meta", "payload": {
        "id": spec.session_id, "timestamp": at(0), "cwd": spec.cwd.to_string_lossy(),
        "originator": "codex_cli_rs", "cli_version": "0.0.0-synthetic", "git": {"branch": "main"}}}),
    ];
    let mut step = 0;
    let item =
        |p: Value, step: i64| json!({"timestamp": at(step), "type": "response_item", "payload": p});
    for t in &spec.turns {
        step += 1;
        lines.push(item(json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": t.user}]}), step));
        for tool in &t.tools {
            step += 1;
            lines.push(item(json!({"type": "reasoning", "summary": [], "content": [{"type": "reasoning_text", "text": HIDDEN_REASONING}], "encrypted_content": "synthetic"}), step));
            match tool.kind {
                ToolKind::Shell => {
                    let args = json!({"command": ["bash", "-lc", tool.command]}).to_string();
                    lines.push(item(json!({"type": "function_call", "name": "shell", "arguments": args, "call_id": tool.tool_use_id}), step));
                    let out = json!({"output": format!("{}{}", tool.stdout, tool.stderr), "metadata": {"exit_code": tool.exit_code, "duration_seconds": 0}}).to_string();
                    lines.push(item(json!({"type": "function_call_output", "call_id": tool.tool_use_id, "output": out}), step));
                }
                _ => {
                    let verb = if tool.kind == ToolKind::Write {
                        "Add"
                    } else {
                        "Update"
                    };
                    let patch = format!(
                        "*** Begin Patch\n*** {verb} File: {}\n+synthetic\n*** End Patch\n",
                        tool.file
                    );
                    lines.push(item(json!({"type": "custom_tool_call", "name": "apply_patch", "input": patch, "call_id": tool.tool_use_id}), step));
                    lines.push(item(json!({"type": "custom_tool_call_output", "call_id": tool.tool_use_id, "output": "Success. Updated files."}), step));
                }
            }
        }
        step += 1;
        lines.push(item(json!({"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": t.assistant}]}), step));
    }
    put(&path, &jsonl(&lines))?;
    Ok(path)
}

fn write_gemini(home: &Path, spec: &SessionSpec) -> std::io::Result<PathBuf> {
    let proj = home.join(".gemini/tmp/synthetic-project");
    put(
        &proj.join(".project_root"),
        spec.cwd.to_string_lossy().as_bytes(),
    )?;
    let path = proj
        .join("chats")
        .join(format!("session-{}.json", spec.session_id));
    let mut messages = Vec::new();
    let mut step = 0;
    for t in &spec.turns {
        step += 1;
        messages.push(json!({"id": format!("m{step}"), "timestamp": at(step), "type": "user", "content": t.user}));
        step += 1;
        let calls: Vec<Value> = t
            .tools
            .iter()
            .map(|tool| {
                let (name, args) = match tool.kind {
                    ToolKind::Shell => ("run_shell_command", json!({"command": tool.command})),
                    ToolKind::Write => ("write_file", json!({"file_path": abs(&spec.cwd, &tool.file), "content": "synthetic\n"})),
                    ToolKind::Edit => ("replace", json!({"file_path": abs(&spec.cwd, &tool.file), "old_string": "a", "new_string": "b"})),
                };
                let display = if tool.kind == ToolKind::Shell { tool.stdout.clone() } else { "ok".into() };
                json!({"id": tool.tool_use_id, "name": name, "args": args, "status": if tool.exit_code == 0 { "success" } else { "error" },
                       "timestamp": at(step), "resultDisplay": display,
                       "result": [{"functionResponse": {"id": tool.tool_use_id, "name": name, "response": {"output": display}}}]})
            })
            .collect();
        messages.push(json!({"id": format!("m{step}"), "timestamp": at(step), "type": "gemini", "content": t.assistant,
                             "thoughts": [{"subject": "plan", "description": HIDDEN_REASONING, "timestamp": at(step)}],
                             "toolCalls": calls}));
    }
    let doc = json!({"sessionId": spec.session_id, "projectHash": "synthetic", "startTime": at(0), "lastUpdated": at(step), "messages": messages});
    put(&path, serde_json::to_string_pretty(&doc)?.as_bytes())?;
    Ok(path)
}

fn write_opencode(home: &Path, spec: &SessionSpec) -> std::io::Result<PathBuf> {
    let storage = home.join(".local/share/opencode/storage");
    let sid = &spec.session_id;
    let path = storage
        .join("session/proj_synthetic")
        .join(format!("{sid}.json"));
    put(
        &path,
        json!({"id": sid, "projectID": "proj_synthetic", "directory": spec.cwd.to_string_lossy(), "title": "synthetic",
               "time": {"created": at_ms(0), "updated": at_ms(100)}})
        .to_string()
        .as_bytes(),
    )?;
    let mut step = 0;
    let msg = |role: &str, parts: Vec<Value>, step: i64| -> std::io::Result<()> {
        let mid = format!("msg_{step:04}");
        put(
            &storage
                .join("message")
                .join(sid)
                .join(format!("{mid}.json")),
            json!({"id": mid, "sessionID": sid, "role": role, "time": {"created": at_ms(step)}})
                .to_string()
                .as_bytes(),
        )?;
        for (i, p) in parts.into_iter().enumerate() {
            let mut p = p;
            p["id"] = json!(format!("prt_{step:04}_{i}"));
            p["messageID"] = json!(mid);
            p["sessionID"] = json!(sid);
            put(
                &storage
                    .join("part")
                    .join(&mid)
                    .join(format!("prt_{step:04}_{i}.json")),
                p.to_string().as_bytes(),
            )?;
        }
        Ok(())
    };
    for t in &spec.turns {
        step += 1;
        msg("user", vec![json!({"type": "text", "text": t.user})], step)?;
        step += 1;
        let mut parts = vec![json!({"type": "reasoning", "text": HIDDEN_REASONING})];
        for tool in &t.tools {
            let (name, input) = match tool.kind {
                ToolKind::Shell => ("bash", json!({"command": tool.command})),
                ToolKind::Write => (
                    "write",
                    json!({"filePath": abs(&spec.cwd, &tool.file), "content": "synthetic\n"}),
                ),
                ToolKind::Edit => (
                    "edit",
                    json!({"filePath": abs(&spec.cwd, &tool.file), "oldString": "a", "newString": "b"}),
                ),
            };
            let out = if tool.kind == ToolKind::Shell {
                tool.stdout.clone()
            } else {
                "ok".into()
            };
            parts.push(json!({"type": "tool", "tool": name, "callID": tool.tool_use_id, "state": {
                "status": if tool.exit_code == 0 { "completed" } else { "error" }, "input": input, "output": out,
                "metadata": {"exit": tool.exit_code}, "time": {"start": at_ms(step), "end": at_ms(step)}}}));
        }
        parts.push(json!({"type": "text", "text": t.assistant}));
        msg("assistant", parts, step)?;
    }
    Ok(path)
}

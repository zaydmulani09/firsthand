//! Shared parsing of harness hook JSON into [`HookEvent`].

use crate::util::{join_argv, s, summarize_input};
use fh_core::{AgentId, Error, FileOp, HookEvent, HookEventKind, Result, ToolCall, ToolClass};
use serde_json::Value;

/// Hook input above this size is refused (harness payloads are far smaller).
const MAX_INPUT: usize = 32 * 1024 * 1024;

/// Normalized kind of a harness event name, across agents.
pub(crate) fn kind_of(name: &str) -> HookEventKind {
    use HookEventKind as K;
    match name {
        "SessionStart" | "session.created" => K::SessionStart,
        "PreToolUse" | "BeforeTool" | "tool.execute.before" => K::PreToolUse,
        "PostToolUse" | "PostToolUseFailure" | "AfterTool" | "tool.execute.after" => K::PostToolUse,
        "Stop" | "AfterAgent" | "session.idle" | "agent-turn-complete" => K::Stop,
        "SubagentStop" => K::SubagentStop,
        "PreCompact" | "PreCompress" | "session.compacted" => K::PreCompact,
        "SessionEnd" => K::SessionEnd,
        "UserPromptSubmit" | "BeforeAgent" => K::UserPromptSubmit,
        _ => K::Other,
    }
}

/// Decode the JSON payload: stdin, or the last argv entry when stdin is empty
/// (Codex legacy `notify`).
pub(crate) fn payload(raw: &[u8], argv_tail: &[String]) -> Result<Value> {
    if raw.len() > MAX_INPUT {
        return Err(Error::Invalid {
            what: "hook input",
            value: format!("{} bytes", raw.len()),
            reason: "larger than 32 MiB".into(),
        });
    }
    let text = String::from_utf8_lossy(raw);
    let text = if text.trim().is_empty() {
        match argv_tail.last() {
            Some(a) if a.trim_start().starts_with('{') => a.clone().into(),
            _ => {
                return Err(Error::Invalid {
                    what: "hook input",
                    value: String::new(),
                    reason: "no JSON on stdin or in the last argument".into(),
                });
            }
        }
    } else {
        text
    };
    let v: Value = serde_json::from_str(text.trim()).map_err(|e| Error::Invalid {
        what: "hook input",
        value: String::new(),
        reason: format!("not JSON: {e}"),
    })?;
    if !v.is_object() {
        return Err(Error::Invalid {
            what: "hook input",
            value: String::new(),
            reason: "expected a JSON object".into(),
        });
    }
    Ok(v)
}

/// Event fields shared by Claude Code, Codex and Gemini CLI payloads.
pub(crate) fn base_event(agent: AgentId, v: &Value, argv_tail: &[String]) -> HookEvent {
    let raw_event_name = s(v, "hook_event_name")
        .or_else(|| s(v, "type"))
        .or_else(|| {
            argv_tail
                .first()
                .map(String::as_str)
                .filter(|a| !a.starts_with('{'))
        })
        .unwrap_or("")
        .to_owned();
    let str_of = |keys: &[&str]| keys.iter().find_map(|k| s(v, k)).map(str::to_owned);
    HookEvent {
        agent,
        kind: kind_of(&raw_event_name),
        raw_event_name,
        session_id: str_of(&["session_id", "sessionId", "thread-id", "thread_id"]),
        tool_use_id: str_of(&["tool_use_id", "call_id", "callID"]),
        cwd: str_of(&["cwd"]),
        transcript_path: str_of(&["transcript_path"]),
        source: str_of(&["source"]),
        tool: None,
        agent_version: str_of(&["agent_version", "version"]),
        model: match v.get("model") {
            Some(Value::String(m)) => Some(m.clone()),
            Some(Value::Object(o)) => o
                .get("id")
                .or_else(|| o.get("display_name"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            _ => None,
        },
    }
}

/// An empty tool call of a class.
pub(crate) fn tool(name: &str, class: ToolClass) -> ToolCall {
    ToolCall {
        name: name.to_owned(),
        class,
        command: None,
        argv: None,
        file_paths: Vec::new(),
        file_op: None,
        exit_code: None,
        stdout: None,
        stderr: None,
        is_error: None,
        summary: None,
    }
}

/// Shell command from a tool input: a string, or an argv array.
pub(crate) fn shell_command(input: &Value) -> (Option<String>, Option<Vec<String>>) {
    match input.get("command").or_else(|| input.get("cmd")) {
        Some(Value::String(c)) => (Some(c.clone()), None),
        Some(Value::Array(a)) => {
            let argv: Vec<String> = a
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            // `bash -lc "<script>"` style: the script is the command text.
            let text = match argv.as_slice() {
                [sh, flag, script]
                    if flag.starts_with('-') && flag.ends_with('c') && is_shell(sh) =>
                {
                    script.clone()
                }
                _ => join_argv(a),
            };
            (Some(text), Some(argv))
        }
        _ => (None, None),
    }
}

fn is_shell(p: &str) -> bool {
    let name = p
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(p)
        .to_ascii_lowercase();
    matches!(
        name.trim_end_matches(".exe"),
        "bash" | "sh" | "zsh" | "pwsh" | "powershell" | "cmd"
    )
}

/// `Exit code N` (or `Exit code: N`, `Exit Code: N`) at the start of a line.
pub(crate) fn exit_code_in(text: &str) -> Option<i32> {
    text.lines().find_map(|l| {
        let l = l.trim();
        let rest = l
            .strip_prefix("Exit code")
            .or_else(|| l.strip_prefix("Exit Code"))?;
        rest.trim_start_matches(':')
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    })
}

/// Shell result from a tool response: an object with exit code and streams, or text
/// in the `Exit code: N ... Output:` shape. Returns (exit code, stdout, stderr).
pub(crate) fn shell_result(resp: &Value) -> (Option<i32>, Option<String>, Option<String>) {
    match resp {
        Value::Object(_) => {
            let code = ["exit_code", "exitCode", "returnCode", "exit"]
                .iter()
                .find_map(|k| resp.get(*k).and_then(Value::as_i64))
                .and_then(|c| i32::try_from(c).ok());
            let out = ["stdout", "output", "aggregated_output"]
                .iter()
                .find_map(|k| s(resp, k))
                .map(str::to_owned);
            let err = s(resp, "stderr").map(str::to_owned);
            match (code, &out) {
                (None, Some(o)) => {
                    let (c, so) = split_exec_text(o);
                    (c, so.or(out), err)
                }
                _ => (code, out, err),
            }
        }
        Value::String(t) => {
            let (c, out) = split_exec_text(t);
            (c, out.or_else(|| Some(t.clone())), None)
        }
        _ => (None, None, None),
    }
}

/// Split `Exit code: N\n...\nOutput:\n<text>` into its code and output.
fn split_exec_text(t: &str) -> (Option<i32>, Option<String>) {
    let code = exit_code_in(t.lines().take(3).collect::<Vec<_>>().join("\n").as_str());
    let out = t
        .split_once("Output:\n")
        .map(|(_, o)| o.to_owned())
        .filter(|_| code.is_some());
    (code, out)
}

/// File paths and operation from an `apply_patch` envelope.
pub(crate) fn patch_files(patch: &str) -> (Vec<String>, Option<FileOp>) {
    let mut paths = Vec::new();
    let mut ops = Vec::new();
    for l in patch.lines() {
        let l = l.trim_end();
        for (prefix, op) in [
            ("*** Add File: ", FileOp::Write),
            ("*** Update File: ", FileOp::Edit),
            ("*** Delete File: ", FileOp::Delete),
            ("*** Move to: ", FileOp::Rename),
        ] {
            if let Some(p) = l.strip_prefix(prefix) {
                paths.push(p.trim().to_owned());
                ops.push(op);
            }
        }
    }
    paths.dedup();
    let op = match ops.as_slice() {
        [] => None,
        [first, rest @ ..] if rest.iter().all(|o| o == first) => Some(*first),
        _ if ops.contains(&FileOp::Rename) => Some(FileOp::Rename),
        _ => Some(FileOp::Edit),
    };
    (paths, op)
}

/// First string anywhere in a JSON value that contains `needle`.
pub(crate) fn find_string<'a>(v: &'a Value, needle: &str) -> Option<&'a str> {
    match v {
        Value::String(t) if t.contains(needle) => Some(t),
        Value::Array(a) => a.iter().find_map(|x| find_string(x, needle)),
        Value::Object(o) => o.values().find_map(|x| find_string(x, needle)),
        _ => None,
    }
}

/// First string value under `key` anywhere in a JSON value.
pub(crate) fn find_key<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    match v {
        Value::Object(o) => o
            .get(key)
            .and_then(Value::as_str)
            .or_else(|| o.values().find_map(|x| find_key(x, key))),
        Value::Array(a) => a.iter().find_map(|x| find_key(x, key)),
        _ => None,
    }
}

/// Generic "other tool" call with a summary.
pub(crate) fn other_tool(name: &str, input: &Value) -> ToolCall {
    let mut t = tool(name, ToolClass::Other);
    let sum = summarize_input(input);
    t.summary = Some(if sum.is_empty() { name.to_owned() } else { sum });
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exit_codes() {
        assert_eq!(exit_code_in("Exit code 1\nError: x"), Some(1));
        assert_eq!(exit_code_in("Exit code: 127"), Some(127));
        assert_eq!(exit_code_in("Command: ls\nExit Code: 0"), Some(0));
        assert_eq!(exit_code_in("no code here"), None);
        let (c, o, _) = shell_result(&json!(
            "Exit code: 2\nWall time: 0.1 seconds\nOutput:\nboom\n"
        ));
        assert_eq!((c, o.as_deref()), (Some(2), Some("boom\n")));
    }

    #[test]
    fn patches() {
        let (p, op) = patch_files(
            "*** Begin Patch\n*** Add File: a.rs\n+x\n*** Update File: b.rs\n*** End Patch",
        );
        assert_eq!(p, vec!["a.rs", "b.rs"]);
        assert_eq!(op, Some(FileOp::Edit));
        let (_, op) = patch_files("*** Delete File: c.rs\n");
        assert_eq!(op, Some(FileOp::Delete));
    }

    #[test]
    fn shell_argv_forms() {
        let (c, a) = shell_command(&json!({"command": ["bash", "-lc", "cargo test -q"]}));
        assert_eq!(c.as_deref(), Some("cargo test -q"));
        assert_eq!(a.unwrap().len(), 3);
        let (c, a) = shell_command(&json!({"command": ["git", "commit", "-m", "a b"]}));
        assert_eq!(c.as_deref(), Some("git commit -m \"a b\""));
        assert!(a.is_some());
    }
}

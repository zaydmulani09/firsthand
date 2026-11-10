//! Hook and MCP plans (byte-exact install/uninstall round trips, idempotence,
//! scopes), hook event parsing for every agent, launch specs, and detection.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use fh_adapters::{
    AgentAdapter, BriefRef, ClaudeCode, Codex, GeminiCli, Generic, HookPlan, HookScope, HostEnv,
    OpenCode, RepoCtx, apply_plan, get_with, is_firsthand_hook_command, plugin_source,
    unified_diff, uninstall_plan_with,
};
use fh_core::{AgentId, FileOp, HookEventKind, ToolClass};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

struct Env {
    _home: tempfile::TempDir,
    _repo: tempfile::TempDir,
    host: HostEnv,
    repo: RepoCtx,
}

fn env() -> Env {
    let home = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    Env {
        host: HostEnv::with_home(home.path()),
        repo: RepoCtx {
            root: repo.path().to_path_buf(),
            common_dir: repo.path().join(".git"),
            firsthand_cmd: "firsthand".into(),
        },
        _home: home,
        _repo: repo,
    }
}

fn adapter(e: &Env, id: AgentId) -> Box<dyn AgentAdapter> {
    get_with(id, e.host.clone())
}

fn write(p: &Path, s: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}

fn apply(plan: &HookPlan) {
    for e in &plan.edits {
        let d = unified_diff(
            &e.path.to_string_lossy(),
            e.before.as_deref().unwrap_or(b""),
            &e.after,
        );
        assert!(!d.is_empty(), "edit without a diff: {}", e.path.display());
    }
    apply_plan(plan).unwrap();
}

fn uninstall(e: &Env, scope: HookScope) {
    for p in uninstall_plan_with(&e.repo, scope, &e.host).unwrap() {
        apply_plan(&p).unwrap();
    }
}

/// Install hooks and MCP for `id`, check idempotence, uninstall, and compare bytes.
fn round_trip(e: &Env, id: AgentId, files: &[(PathBuf, Option<&str>)]) {
    for (p, content) in files {
        if let Some(c) = content {
            write(p, c);
        }
    }
    let a = adapter(e, id);
    apply(&a.hook_plan(&e.repo, HookScope::Repo).unwrap());
    apply(&a.mcp_install_plan(&e.repo, HookScope::Repo).unwrap());
    // Second run changes nothing.
    assert!(
        a.hook_plan(&e.repo, HookScope::Repo)
            .unwrap()
            .edits
            .is_empty()
    );
    assert!(
        a.mcp_install_plan(&e.repo, HookScope::Repo)
            .unwrap()
            .edits
            .is_empty()
    );
    for (p, _) in files {
        let text = std::fs::read_to_string(p).unwrap();
        assert!(text.contains("firsthand"), "{}: {text}", p.display());
    }
    uninstall(e, HookScope::Repo);
    for (p, content) in files {
        match content {
            Some(c) => assert_eq!(std::fs::read_to_string(p).unwrap(), *c, "{}", p.display()),
            None => assert!(!p.exists(), "{} should be removed", p.display()),
        }
    }
    // Nothing left to uninstall.
    assert!(
        uninstall_plan_with(&e.repo, HookScope::Repo, &e.host)
            .unwrap()
            .is_empty()
    );
}

const USER_SETTINGS: &str = "{\n    \"permissions\": {\n        \"allow\": [\"Bash(ls)\"]\n    },\n    \"hooks\": {\n        \"Stop\": [\n            {\"hooks\": [{\"type\": \"command\", \"command\": \"say done\"}]}\n        ]\n    }\n}";

#[test]
fn claude_round_trip_existing_files() {
    let e = env();
    let settings = e.repo.root.join(".claude/settings.local.json");
    let mcp = e.repo.root.join(".mcp.json");
    round_trip(
        &e,
        AgentId::ClaudeCode,
        &[
            (settings.clone(), Some(USER_SETTINGS)),
            (
                mcp,
                Some(
                    "{\r\n  \"mcpServers\": {\r\n    \"other\": {\"command\": \"x\"}\r\n  }\r\n}\r\n",
                ),
            ),
        ],
    );
}

#[test]
fn claude_round_trip_missing_files() {
    let e = env();
    round_trip(
        &e,
        AgentId::ClaudeCode,
        &[
            (e.repo.root.join(".claude/settings.local.json"), None),
            (e.repo.root.join(".mcp.json"), None),
        ],
    );
}

#[test]
fn claude_hook_entries_shape() {
    let e = env();
    apply(
        &adapter(&e, AgentId::ClaudeCode)
            .hook_plan(&e.repo, HookScope::Repo)
            .unwrap(),
    );
    let v: Value = serde_json::from_slice(
        &std::fs::read(e.repo.root.join(".claude/settings.local.json")).unwrap(),
    )
    .unwrap();
    let pre = &v["hooks"]["PreToolUse"][0];
    assert_eq!(pre["matcher"], "Bash");
    assert_eq!(pre["hooks"][0]["type"], "command");
    assert_eq!(
        pre["hooks"][0]["command"],
        "firsthand hook claude-code PreToolUse"
    );
    assert_eq!(
        v["hooks"]["SessionStart"][0]["matcher"],
        "startup|resume|clear|compact"
    );
    assert!(v["hooks"]["Stop"][0].get("matcher").is_none());
    for ev in [
        "PostToolUse",
        "PostToolUseFailure",
        "SubagentStop",
        "PreCompact",
        "SessionEnd",
    ] {
        assert!(v["hooks"][ev].is_array(), "{ev}");
    }
    // Every injected command carries the uninstall marker.
    for (_, groups) in v["hooks"].as_object().unwrap() {
        for g in groups.as_array().unwrap() {
            assert!(is_firsthand_hook_command(
                g["hooks"][0]["command"].as_str().unwrap()
            ));
        }
    }
}

#[test]
fn changed_command_replaces_entries() {
    let mut e = env();
    let a = adapter(&e, AgentId::ClaudeCode);
    apply(&a.hook_plan(&e.repo, HookScope::Repo).unwrap());
    e.repo.firsthand_cmd = "\"C:\\Program Files\\Firsthand\\firsthand.exe\"".into();
    apply(&a.hook_plan(&e.repo, HookScope::Repo).unwrap());
    let text = std::fs::read_to_string(e.repo.root.join(".claude/settings.local.json")).unwrap();
    assert_eq!(
        text.matches("hook claude-code PreToolUse").count(),
        1,
        "{text}"
    );
    assert!(text.contains("firsthand.exe\\\" hook claude-code"));
}

#[test]
fn user_scope_writes_only_under_home() {
    let e = env();
    let a = adapter(&e, AgentId::ClaudeCode);
    let plan = a.hook_plan(&e.repo, HookScope::User).unwrap();
    assert!(
        plan.edits
            .iter()
            .all(|x| x.path.starts_with(e.host.home.as_ref().unwrap()))
    );
    assert!(plan.edits.iter().all(|x| x.scope == HookScope::User));
    apply(&plan);
    assert!(!e.repo.root.join(".claude").exists());
    // Repo-scope uninstall leaves user files alone; user-scope uninstall removes them.
    uninstall(&e, HookScope::Repo);
    assert!(
        e.host
            .home
            .as_ref()
            .unwrap()
            .join(".claude/settings.json")
            .exists()
    );
    uninstall(&e, HookScope::User);
    assert!(
        !e.host
            .home
            .as_ref()
            .unwrap()
            .join(".claude/settings.json")
            .exists()
    );
}

#[test]
fn stale_plan_is_refused() {
    let e = env();
    let p = e.repo.root.join(".claude/settings.local.json");
    write(&p, "{}\n");
    let plan = adapter(&e, AgentId::ClaudeCode)
        .hook_plan(&e.repo, HookScope::Repo)
        .unwrap();
    write(&p, "{\"model\": \"changed\"}\n");
    let err = apply_plan(&plan).unwrap_err().to_string();
    assert!(err.contains("changed since the plan was made"), "{err}");
    assert_eq!(
        std::fs::read_to_string(&p).unwrap(),
        "{\"model\": \"changed\"}\n"
    );
}

#[test]
fn apply_writes_timestamped_backup() {
    let e = env();
    let p = e.repo.root.join(".claude/settings.local.json");
    write(&p, USER_SETTINGS);
    let backups = apply_plan(
        &adapter(&e, AgentId::ClaudeCode)
            .hook_plan(&e.repo, HookScope::Repo)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(backups.len(), 1);
    let name = backups[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(
        name.starts_with("settings.local.json.firsthand-") && name.ends_with(".bak"),
        "{name}"
    );
    assert_eq!(std::fs::read_to_string(&backups[0]).unwrap(), USER_SETTINGS);
}

#[test]
fn malformed_config_is_reported() {
    let e = env();
    write(
        &e.repo.root.join(".claude/settings.local.json"),
        "{\"hooks\": [}",
    );
    let err = adapter(&e, AgentId::ClaudeCode)
        .hook_plan(&e.repo, HookScope::Repo)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("settings.local.json") && err.contains("fix the file"),
        "{err}"
    );
}

#[test]
fn codex_round_trip() {
    let e = env();
    round_trip(
        &e,
        AgentId::Codex,
        &[
            (
                e.repo.root.join(".codex/hooks.json"),
                Some(
                    "{\n  \"description\": \"team hooks\",\n  \"hooks\": {\"Stop\": [{\"hooks\": [{\"type\": \"command\", \"command\": \"say done\"}]}]}\n}\n",
                ),
            ),
            (
                e.repo.root.join(".codex/config.toml"),
                Some("model = \"synthetic\"\n\n[mcp_servers.other]\ncommand = \"x\"\n"),
            ),
        ],
    );
}

#[test]
fn codex_plans_and_trust_step() {
    let e = env();
    let a = adapter(&e, AgentId::Codex);
    let hp = a.hook_plan(&e.repo, HookScope::Repo).unwrap();
    assert!(hp.manual_steps.iter().any(|s| s.contains("/hooks")));
    apply(&hp);
    apply(&a.mcp_install_plan(&e.repo, HookScope::Repo).unwrap());
    let toml_text = std::fs::read_to_string(e.repo.root.join(".codex/config.toml")).unwrap();
    let t: toml::Table = toml::from_str(&toml_text).unwrap();
    assert_eq!(
        t["mcp_servers"]["firsthand"]["args"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let hooks: Value =
        serde_json::from_slice(&std::fs::read(e.repo.root.join(".codex/hooks.json")).unwrap())
            .unwrap();
    assert_eq!(hooks["hooks"]["PreToolUse"][0]["matcher"], "^Bash$");
    // A user-defined server of the same name is never overwritten.
    let e2 = env();
    write(
        &e2.repo.root.join(".codex/config.toml"),
        "[mcp_servers.firsthand]\ncommand = \"mine\"\n",
    );
    let p = adapter(&e2, AgentId::Codex)
        .mcp_install_plan(&e2.repo, HookScope::Repo)
        .unwrap();
    assert!(p.edits.is_empty());
    assert!(p.manual_steps.iter().any(|s| s.contains("left unchanged")));
}

#[test]
fn gemini_round_trip_with_comments() {
    let e = env();
    round_trip(
        &e,
        AgentId::GeminiCli,
        &[(
            e.repo.root.join(".gemini/settings.json"),
            Some(
                "{\n  // team settings\n  \"theme\": \"Default\",\n  \"mcpServers\": {\"other\": {\"command\": \"x\"}}\n}\n",
            ),
        )],
    );
}

#[test]
fn gemini_hook_shape() {
    let e = env();
    apply(
        &adapter(&e, AgentId::GeminiCli)
            .hook_plan(&e.repo, HookScope::Repo)
            .unwrap(),
    );
    let v: Value =
        serde_json::from_slice(&std::fs::read(e.repo.root.join(".gemini/settings.json")).unwrap())
            .unwrap();
    assert_eq!(v["hooks"]["BeforeTool"][0]["matcher"], "run_shell_command");
    assert_eq!(v["hooks"]["AfterTool"][0]["hooks"][0]["name"], "firsthand");
    assert_eq!(
        v["hooks"]["PreCompress"][0]["hooks"][0]["command"],
        "firsthand hook gemini-cli PreCompress"
    );
}

#[test]
fn opencode_round_trip_and_plugin() {
    let e = env();
    round_trip(
        &e,
        AgentId::Opencode,
        &[
            (e.repo.root.join(".opencode/plugins/firsthand.ts"), None),
            (
                e.repo.root.join("opencode.json"),
                Some("{\n  \"$schema\": \"https://opencode.ai/config.json\"\n}\n"),
            ),
        ],
    );
    let a = adapter(&e, AgentId::Opencode);
    apply(&a.hook_plan(&e.repo, HookScope::Repo).unwrap());
    let plugin =
        std::fs::read_to_string(e.repo.root.join(".opencode/plugins/firsthand.ts")).unwrap();
    assert_eq!(plugin, plugin_source("firsthand"));
    for hook in [
        "tool.execute.before",
        "tool.execute.after",
        "session.created",
        "session.idle",
        "session.compacted",
        "file.edited",
    ] {
        assert!(plugin.contains(hook), "{hook}");
    }
    // A foreign file at the plugin path is refused, never overwritten.
    let e2 = env();
    write(
        &e2.repo.root.join(".opencode/plugins/firsthand.ts"),
        "export const Mine = 1\n",
    );
    let err = adapter(&e2, AgentId::Opencode)
        .hook_plan(&e2.repo, HookScope::Repo)
        .unwrap_err();
    assert!(err.to_string().contains("not written by firsthand"));
    assert!(
        uninstall_plan_with(&e2.repo, HookScope::Repo, &e2.host)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn generic_has_no_edits() {
    let e = env();
    let g = Generic;
    assert!(
        g.hook_plan(&e.repo, HookScope::Repo)
            .unwrap()
            .edits
            .is_empty()
    );
    let m = g.mcp_install_plan(&e.repo, HookScope::Repo).unwrap();
    assert!(m.edits.is_empty() && m.manual_steps[0].contains("firsthand mcp serve"));
    assert!(g.parse_hook_event(b"{}", &[]).is_err());
    assert!(g.discover_sessions(&e.repo).unwrap().is_empty());
}

// ------------------------------------------------------------ hook events

fn parse(id: AgentId, payload: Value, argv: &[&str]) -> fh_core::HookEvent {
    let argv: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    get_with(id, HostEnv::default())
        .parse_hook_event(payload.to_string().as_bytes(), &argv)
        .unwrap()
}

#[test]
fn claude_events() {
    let base = json!({"session_id": "s1", "transcript_path": "/t.jsonl", "cwd": "/w", "permission_mode": "default", "unknown_future_field": {"a": 1}});
    let with = |extra: Value| {
        let mut v = base.clone();
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        v
    };
    let ev = parse(
        AgentId::ClaudeCode,
        with(json!({"hook_event_name": "SessionStart", "source": "resume", "model": "synthetic"})),
        &["SessionStart"],
    );
    assert_eq!(
        (ev.kind, ev.source.as_deref(), ev.session_id.as_deref()),
        (HookEventKind::SessionStart, Some("resume"), Some("s1"))
    );
    assert_eq!(ev.transcript_path.as_deref(), Some("/t.jsonl"));

    let pre = parse(
        AgentId::ClaudeCode,
        with(
            json!({"hook_event_name": "PreToolUse", "tool_name": "Bash", "tool_use_id": "toolu_1", "tool_input": {"command": "cargo test", "description": "x"}}),
        ),
        &["PreToolUse"],
    );
    let t = pre.tool.unwrap();
    assert_eq!(
        (pre.kind, t.class, t.command.as_deref(), t.exit_code),
        (
            HookEventKind::PreToolUse,
            ToolClass::Shell,
            Some("cargo test"),
            None
        )
    );
    assert_eq!(pre.tool_use_id.as_deref(), Some("toolu_1"));

    let post = parse(
        AgentId::ClaudeCode,
        with(
            json!({"hook_event_name": "PostToolUse", "tool_name": "Bash", "tool_use_id": "toolu_1", "tool_input": {"command": "cargo test"}, "tool_response": {"stdout": "test result: ok. 3 passed", "stderr": "", "interrupted": false, "isImage": false}}),
        ),
        &["PostToolUse"],
    );
    let t = post.tool.unwrap();
    assert_eq!(
        (t.exit_code, t.is_error, t.stdout.as_deref()),
        (Some(0), Some(false), Some("test result: ok. 3 passed"))
    );

    let fail = parse(
        AgentId::ClaudeCode,
        with(
            json!({"hook_event_name": "PostToolUseFailure", "tool_name": "Bash", "tool_use_id": "toolu_2", "tool_input": {"command": "npm test"}, "error": "Exit code 1\nError: Cannot find module", "is_interrupt": false}),
        ),
        &["PostToolUseFailure"],
    );
    let t = fail.tool.unwrap();
    assert_eq!(fail.kind, HookEventKind::PostToolUse);
    assert_eq!(
        (t.exit_code, t.is_error, t.stderr.as_deref()),
        (Some(1), Some(true), Some("Error: Cannot find module"))
    );

    let w = parse(
        AgentId::ClaudeCode,
        with(
            json!({"hook_event_name": "PostToolUse", "tool_name": "Write", "tool_input": {"file_path": "/w/src/a.rs", "content": "x"}, "tool_response": {"filePath": "/w/src/a.rs", "type": "create"}}),
        ),
        &["PostToolUse"],
    );
    let t = w.tool.unwrap();
    assert_eq!(
        (t.class, t.file_op, t.file_paths),
        (
            ToolClass::FileEdit,
            Some(FileOp::Write),
            vec!["/w/src/a.rs".to_owned()]
        )
    );

    let other = parse(
        AgentId::ClaudeCode,
        with(
            json!({"hook_event_name": "PostToolUse", "tool_name": "Grep", "tool_input": {"pattern": "fn main"}, "tool_response": {}}),
        ),
        &["PostToolUse"],
    );
    assert_eq!(other.tool.unwrap().summary.as_deref(), Some("fn main"));

    // The event name falls back to argv when the payload lacks it.
    let stop = parse(AgentId::ClaudeCode, json!({"session_id": "s1"}), &["Stop"]);
    assert_eq!(stop.kind, HookEventKind::Stop);
    for (name, kind) in [
        ("PreCompact", HookEventKind::PreCompact),
        ("SessionEnd", HookEventKind::SessionEnd),
        ("SubagentStop", HookEventKind::SubagentStop),
        ("Notification", HookEventKind::Other),
    ] {
        assert_eq!(
            parse(
                AgentId::ClaudeCode,
                json!({"hook_event_name": name}),
                &[name]
            )
            .kind,
            kind
        );
    }
}

#[test]
fn bad_hook_input_is_an_error() {
    let a = get_with(AgentId::ClaudeCode, HostEnv::default());
    assert!(a.parse_hook_event(b"not json", &["Stop".into()]).is_err());
    assert!(a.parse_hook_event(b"[1,2]", &["Stop".into()]).is_err());
    assert!(a.parse_hook_event(b"", &["Stop".into()]).is_err());
}

#[test]
fn codex_events() {
    let post = parse(
        AgentId::Codex,
        json!({"session_id": "c1", "turn_id": "t1", "cwd": "/w", "hook_event_name": "PostToolUse", "model": "synthetic", "tool_name": "Bash", "tool_use_id": "call_9", "tool_input": {"command": ["bash", "-lc", "cargo test"]}, "tool_response": "Exit code: 101\nWall time: 2.0 seconds\nOutput:\ntest result: FAILED. 1 passed; 1 failed\n"}),
        &["PostToolUse"],
    );
    let t = post.tool.unwrap();
    assert_eq!(t.command.as_deref(), Some("cargo test"));
    assert_eq!(t.argv.as_ref().unwrap().len(), 3);
    assert_eq!((t.exit_code, t.is_error), (Some(101), Some(true)));
    assert_eq!(
        t.stdout.as_deref(),
        Some("test result: FAILED. 1 passed; 1 failed\n")
    );
    assert_eq!(post.model.as_deref(), Some("synthetic"));

    let patch = parse(
        AgentId::Codex,
        json!({"hook_event_name": "PostToolUse", "tool_name": "apply_patch", "tool_input": {"command": "*** Begin Patch\n*** Add File: src/new.rs\n+x\n*** End Patch"}, "tool_response": "Success"}),
        &["PostToolUse"],
    );
    let t = patch.tool.unwrap();
    assert_eq!(
        (t.class, t.file_op, t.file_paths),
        (
            ToolClass::FileEdit,
            Some(FileOp::Write),
            vec!["src/new.rs".to_owned()]
        )
    );

    // Legacy notify: JSON as the last argument, empty stdin.
    let notify = json!({"type": "agent-turn-complete", "thread-id": "th1", "turn-id": "t2", "cwd": "/w", "input-messages": ["x"], "last-assistant-message": "y"}).to_string();
    let ev = get_with(AgentId::Codex, HostEnv::default())
        .parse_hook_event(b"", &["Stop".into(), notify])
        .unwrap();
    assert_eq!(
        (ev.kind, ev.session_id.as_deref(), ev.cwd.as_deref()),
        (HookEventKind::Stop, Some("th1"), Some("/w"))
    );
}

#[test]
fn gemini_events() {
    let llm = "Command: cargo test\nDirectory: (root)\nOutput: test result: ok. 2 passed\nError: (none)\nExit Code: 0\nSignal: (none)\nBackground PIDs: (none)\nProcess Group PGID: 77";
    let post = parse(
        AgentId::GeminiCli,
        json!({"session_id": "g1", "transcript_path": "/t.json", "cwd": "/w", "hook_event_name": "AfterTool", "timestamp": "2026-11-09T10:00:00Z", "tool_name": "run_shell_command", "tool_input": {"command": "cargo test"}, "tool_response": {"llmContent": llm, "returnDisplay": "ok"}}),
        &["AfterTool"],
    );
    assert_eq!(post.kind, HookEventKind::PostToolUse);
    let t = post.tool.unwrap();
    assert_eq!(
        (t.exit_code, t.is_error, t.stdout.as_deref(), t.stderr),
        (
            Some(0),
            Some(false),
            Some("test result: ok. 2 passed"),
            None
        )
    );
    let w = parse(
        AgentId::GeminiCli,
        json!({"hook_event_name": "AfterTool", "tool_name": "replace", "tool_input": {"file_path": "/w/a.rs"}, "tool_response": {"llmContent": "ok", "error": {"message": "no match"}}}),
        &["AfterTool"],
    );
    let t = w.tool.unwrap();
    assert_eq!((t.file_op, t.is_error), (Some(FileOp::Edit), Some(true)));
    for (name, kind) in [
        ("BeforeTool", HookEventKind::PreToolUse),
        ("AfterAgent", HookEventKind::Stop),
        ("PreCompress", HookEventKind::PreCompact),
        ("BeforeAgent", HookEventKind::UserPromptSubmit),
        ("SessionStart", HookEventKind::SessionStart),
    ] {
        assert_eq!(
            parse(
                AgentId::GeminiCli,
                json!({"hook_event_name": name}),
                &[name]
            )
            .kind,
            kind
        );
    }
}

#[test]
fn opencode_events() {
    let post = parse(
        AgentId::Opencode,
        json!({"hook_event_name": "tool.execute.after", "cwd": "/w", "session_id": "ses_1", "tool_name": "bash", "tool_use_id": "call_1", "tool_input": {"command": "pytest"}, "tool_output": "1 failed", "tool_title": "pytest", "tool_metadata": {"exit": 1}}),
        &["tool.execute.after"],
    );
    assert_eq!(
        (post.kind, post.tool_use_id.as_deref()),
        (HookEventKind::PostToolUse, Some("call_1"))
    );
    let t = post.tool.unwrap();
    assert_eq!(
        (
            t.command.as_deref(),
            t.exit_code,
            t.is_error,
            t.stdout.as_deref()
        ),
        (Some("pytest"), Some(1), Some(true), Some("1 failed"))
    );
    let fe = parse(
        AgentId::Opencode,
        json!({"hook_event_name": "file.edited", "file": "/w/a.ts"}),
        &["file.edited"],
    );
    assert_eq!(fe.kind, HookEventKind::Other);
    assert_eq!(fe.tool.unwrap().file_paths, vec!["/w/a.ts".to_owned()]);
    for (name, kind) in [
        ("session.created", HookEventKind::SessionStart),
        ("session.idle", HookEventKind::Stop),
        ("session.compacted", HookEventKind::PreCompact),
        ("tool.execute.before", HookEventKind::PreToolUse),
    ] {
        assert_eq!(
            parse(AgentId::Opencode, json!({"hook_event_name": name}), &[name]).kind,
            kind
        );
    }
}

// ------------------------------------------------------------ launch and detect

#[test]
fn launch_specs_are_unverified_and_never_run() {
    let e = env();
    let brief = BriefRef {
        packet_id: "pkt_x".into(),
        brief_path: ".firsthand/briefs/x.md".into(),
        bootstrap: "Read .firsthand/briefs/x.md now.".into(),
    };
    let expect: [(AgentId, &[&str]); 4] = [
        (AgentId::ClaudeCode, &["claude"]),
        (AgentId::Codex, &["codex"]),
        (AgentId::GeminiCli, &["gemini", "-i"]),
        (AgentId::Opencode, &["opencode", "--prompt"]),
    ];
    for (id, prefix) in expect {
        let l = adapter(&e, id).launch_spec(&brief, &e.repo).unwrap();
        assert_eq!(&l.argv[..prefix.len()], prefix);
        assert_eq!(l.argv.last().unwrap(), &brief.bootstrap);
        assert!(!l.verified);
        assert_eq!(l.cwd, e.repo.root);
    }
    assert!(Generic.launch_spec(&brief, &e.repo).is_err());
}

#[test]
fn detect_runs_version_only() {
    let bin = tempfile::tempdir().unwrap();
    let marker = bin.path().join("args.txt");
    if cfg!(windows) {
        std::fs::write(
            bin.path().join("claude.cmd"),
            format!(
                "@echo %*> \"{}\"\r\n@echo 9.9.9 (synthetic)\r\n",
                marker.display()
            ),
        )
        .unwrap();
    } else {
        let p = bin.path().join("claude");
        std::fs::write(
            &p,
            format!(
                "#!/bin/sh\necho \"$@\" > '{}'\necho '9.9.9 (synthetic)'\n",
                marker.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".claude/projects")).unwrap();
    let mut host = HostEnv::with_home(home.path());
    host.path_var = Some(bin.path().as_os_str().to_owned());
    let d = ClaudeCode::new(host.clone()).detect(&host);
    assert!(d.installed, "{d:?}");
    assert_eq!(d.version.as_deref(), Some("9.9.9 (synthetic)"));
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap().trim(),
        "--version"
    );
    assert!(d.config_paths.iter().any(|p| p.ends_with("projects")));
    // Not on PATH: a note says so.
    let none = Codex::new(host.clone()).detect(&host);
    assert!(!none.installed && none.notes[0].contains("not found on PATH"));
    let g = GeminiCli::new(host.clone()).detect(&host);
    assert!(!g.installed);
    let o = OpenCode::new(host.clone()).detect(&host);
    assert!(!o.installed);
}

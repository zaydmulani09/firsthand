//! The fake agent's logs and hook payloads must be read back correctly by the real
//! adapters: discovery, ingestion, and hook parsing for every agent.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use fh_adapters::{HostEnv, IngestOpts, RepoCtx, get_with};
use fh_core::{AgentId, HookEventKind, Role, ToolClass};
use fh_fakeagent::{HIDDEN_REASONING, SessionSpec, ToolKind, hook_payload, script, write_session};

const AGENTS: [AgentId; 4] = [
    AgentId::ClaudeCode,
    AgentId::Codex,
    AgentId::GeminiCli,
    AgentId::Opencode,
];

#[test]
fn sessions_are_discovered_and_ingested() {
    for agent in AGENTS {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let spec = SessionSpec::sample("fake-1", repo.path());
        write_session(agent, home.path(), &spec).unwrap();
        let a = get_with(agent, HostEnv::with_home(home.path()));
        let ctx = RepoCtx {
            root: repo.path().to_path_buf(),
            common_dir: repo.path().join(".git"),
            firsthand_cmd: "firsthand".into(),
        };
        let found = a.discover_sessions(&ctx).unwrap();
        assert_eq!(found.len(), 1, "{agent:?}");
        let s = a.ingest(&found[0], &IngestOpts::default()).unwrap();
        let json = serde_json::to_string(&s).unwrap();
        assert!(
            !json.contains(HIDDEN_REASONING),
            "{agent:?} ingested hidden reasoning"
        );
        assert_eq!(
            s.final_assistant_text, "Wired the parser into main. Build is clean.",
            "{agent:?}"
        );
        assert_eq!(
            s.messages.iter().filter(|m| m.role == Role::User).count(),
            2,
            "{agent:?}"
        );
        assert_eq!(s.tool_events.len(), 3, "{agent:?}: {:?}", s.tool_events);
        assert!(s.tool_events[1].summary.contains("cargo test"), "{agent:?}");
        assert!(
            s.tool_events[1]
                .output
                .as_deref()
                .unwrap()
                .contains("5 passed"),
            "{agent:?}"
        );
        assert_eq!(s.skipped_records, 0, "{agent:?}");
    }
}

#[test]
fn hook_payloads_parse_as_the_intended_events() {
    let cwd = tempfile::tempdir().unwrap();
    for agent in AGENTS {
        let a = get_with(agent, HostEnv::default());
        for spec in script("fake-2", cwd.path(), 8) {
            let name = fh_fakeagent::event_name(agent, &spec).unwrap();
            let raw = hook_payload(agent, &spec).to_string();
            let ev = a
                .parse_hook_event(raw.as_bytes(), &[name.to_owned()])
                .unwrap();
            assert_eq!(ev.kind, spec.kind, "{agent:?} {name}");
            assert_eq!(ev.session_id.as_deref(), Some("fake-2"));
            let Some(t) = &spec.tool else { continue };
            let got = ev.tool.unwrap();
            match t.kind {
                ToolKind::Shell => {
                    assert_eq!(got.class, ToolClass::Shell, "{agent:?}");
                    assert_eq!(
                        got.command.as_deref(),
                        Some(t.command.as_str()),
                        "{agent:?}"
                    );
                    if spec.kind == HookEventKind::PostToolUse {
                        assert_eq!(got.exit_code, Some(t.exit_code), "{agent:?} {name}");
                        assert_eq!(got.is_error, Some(t.exit_code != 0), "{agent:?}");
                    }
                }
                _ => {
                    assert_eq!(got.class, ToolClass::FileEdit, "{agent:?}");
                    assert!(got.file_paths[0].ends_with("lib.rs"), "{agent:?}");
                }
            }
            if agent != AgentId::GeminiCli {
                assert_eq!(
                    ev.tool_use_id.as_deref(),
                    Some(t.tool_use_id.as_str()),
                    "{agent:?}"
                );
            }
        }
    }
}

#[test]
fn codex_notify_parses() {
    let cwd = tempfile::tempdir().unwrap();
    let raw = fh_fakeagent::notify_payload("th-9", cwd.path()).to_string();
    let ev = get_with(AgentId::Codex, HostEnv::default())
        .parse_hook_event(b"", &["Stop".into(), raw])
        .unwrap();
    assert_eq!(
        (ev.kind, ev.session_id.as_deref()),
        (HookEventKind::Stop, Some("th-9"))
    );
}

#[test]
fn invoke_passes_the_payload_on_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let bin = if cfg!(windows) {
        let p = dir.path().join("echo-stdin.cmd");
        std::fs::write(&p, "@echo %*\r\n@more\r\n").unwrap();
        p
    } else {
        let p = dir.path().join("echo-stdin");
        std::fs::write(&p, "#!/bin/sh\necho \"$@\"\ncat\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    };
    let spec = &script("fake-3", dir.path(), 1)[2];
    let out = fh_fakeagent::invoke(&bin, AgentId::ClaudeCode, spec, false).unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("hook claude-code PostToolUseFailure"),
        "{text}"
    );
    assert!(
        text.contains("\"hook_event_name\":\"PostToolUseFailure\""),
        "{text}"
    );
}

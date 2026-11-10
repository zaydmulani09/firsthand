//! `fh-fakeagent`: test-only fake agent.
//!
//! ```text
//! fh-fakeagent session --agent <id> --home <dir> --cwd <dir> [--session-id <s>]
//! fh-fakeagent hook    --firsthand <bin> --agent <id> --event <event> --cwd <dir>
//!                      [--session-id <s>] [--source <s>] [--tool shell|write|edit]
//!                      [--command <text>] [--exit-code <n>] [--stdout <text>]
//!                      [--file <rel>] [--tool-use-id <id>] [--notify]
//! fh-fakeagent script  --firsthand <bin> --agent <id> --cwd <dir> [--session-id <s>] [--pairs <n>]
//! fh-fakeagent payload --agent <id> --event <event> --cwd <dir> [tool flags as for hook]
//! ```
//! Events: session-start, pre-tool, post-tool, stop, subagent-stop, pre-compact,
//! session-end, user-prompt.

use fh_core::{AgentId, HookEventKind};
use fh_fakeagent::{HookSpec, SessionSpec, ToolKind, ToolSpec};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("fh-fakeagent: {e}");
            ExitCode::from(2)
        }
    }
}

struct Args {
    cmd: String,
    kv: BTreeMap<String, String>,
    flags: Vec<String>,
}

impl Args {
    fn get(&self, k: &str) -> Result<&str, String> {
        self.kv
            .get(k)
            .map(String::as_str)
            .ok_or_else(|| format!("missing --{k}"))
    }
    fn opt(&self, k: &str) -> Option<&str> {
        self.kv.get(k).map(String::as_str)
    }
}

fn parse() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let cmd = it
        .next()
        .ok_or("missing command: session, hook, script or payload")?;
    let mut kv = BTreeMap::new();
    let mut flags = Vec::new();
    let rest: Vec<String> = it.collect();
    let mut i = 0;
    while i < rest.len() {
        let k = rest[i]
            .strip_prefix("--")
            .ok_or_else(|| format!("unexpected argument {:?}", rest[i]))?;
        if k == "notify" {
            flags.push(k.to_owned());
            i += 1;
        } else {
            let v = rest
                .get(i + 1)
                .ok_or_else(|| format!("--{k} needs a value"))?;
            kv.insert(k.to_owned(), v.clone());
            i += 2;
        }
    }
    Ok(Args { cmd, kv, flags })
}

fn agent(a: &Args) -> Result<AgentId, String> {
    let s = a.get("agent")?;
    AgentId::parse(s).ok_or_else(|| format!("unknown agent {s:?}"))
}

fn event(s: &str) -> Result<HookEventKind, String> {
    use HookEventKind as K;
    Ok(match s {
        "session-start" => K::SessionStart,
        "pre-tool" => K::PreToolUse,
        "post-tool" => K::PostToolUse,
        "stop" => K::Stop,
        "subagent-stop" => K::SubagentStop,
        "pre-compact" => K::PreCompact,
        "session-end" => K::SessionEnd,
        "user-prompt" => K::UserPromptSubmit,
        _ => return Err(format!("unknown event {s:?}")),
    })
}

fn spec(a: &Args) -> Result<HookSpec, String> {
    let kind = event(a.get("event")?)?;
    let tool = match a.opt("tool") {
        None => None,
        Some(t) => {
            let id = a.opt("tool-use-id").unwrap_or("toolu_fake_0001");
            let mut ts = match t {
                "shell" => ToolSpec::shell(id, a.opt("command").unwrap_or("ls"), 0, ""),
                "write" => {
                    ToolSpec::edit(id, ToolKind::Write, a.opt("file").unwrap_or("src/lib.rs"))
                }
                "edit" => ToolSpec::edit(id, ToolKind::Edit, a.opt("file").unwrap_or("src/lib.rs")),
                other => return Err(format!("unknown tool {other:?}")),
            };
            if let Some(c) = a.opt("exit-code") {
                ts.exit_code = c.parse().map_err(|_| format!("bad --exit-code {c:?}"))?;
            }
            ts.stdout = a.opt("stdout").unwrap_or("").to_owned();
            ts.stderr = a.opt("stderr").unwrap_or("").to_owned();
            Some(ts)
        }
    };
    Ok(HookSpec {
        kind,
        session_id: a.opt("session-id").unwrap_or("fake-session-1").to_owned(),
        cwd: PathBuf::from(a.get("cwd")?),
        source: a
            .opt("source")
            .map(str::to_owned)
            .or_else(|| (kind == HookEventKind::SessionStart).then(|| "startup".into())),
        tool,
    })
}

fn run() -> Result<ExitCode, String> {
    let a = parse()?;
    match a.cmd.as_str() {
        "session" => {
            let s = SessionSpec::sample(
                a.opt("session-id").unwrap_or("fake-session-1"),
                &PathBuf::from(a.get("cwd")?),
            );
            let p = fh_fakeagent::write_session(agent(&a)?, &PathBuf::from(a.get("home")?), &s)
                .map_err(|e| e.to_string())?;
            println!("{}", p.display());
            Ok(ExitCode::SUCCESS)
        }
        "payload" => {
            println!("{}", fh_fakeagent::hook_payload(agent(&a)?, &spec(&a)?));
            Ok(ExitCode::SUCCESS)
        }
        "hook" => {
            let out = fh_fakeagent::invoke(
                &PathBuf::from(a.get("firsthand")?),
                agent(&a)?,
                &spec(&a)?,
                a.flags.iter().any(|f| f == "notify"),
            )
            .map_err(|e| e.to_string())?;
            std::io::stdout()
                .write_all(&out.stdout)
                .map_err(|e| e.to_string())?;
            std::io::stderr()
                .write_all(&out.stderr)
                .map_err(|e| e.to_string())?;
            let code = out.status.code().unwrap_or(1);
            Ok(ExitCode::from(u8::try_from(code).unwrap_or(1)))
        }
        "script" => {
            let ag = agent(&a)?;
            let bin = PathBuf::from(a.get("firsthand")?);
            let pairs: usize = a
                .opt("pairs")
                .unwrap_or("10")
                .parse()
                .map_err(|_| "bad --pairs".to_owned())?;
            let steps = fh_fakeagent::script(
                a.opt("session-id").unwrap_or("fake-session-1"),
                &PathBuf::from(a.get("cwd")?),
                pairs,
            );
            let mut sent = 0u32;
            for s in &steps {
                let out = fh_fakeagent::invoke(&bin, ag, s, false).map_err(|e| e.to_string())?;
                if !out.status.success() {
                    return Err(format!(
                        "firsthand hook exited with {:?}: {}",
                        out.status.code(),
                        String::from_utf8_lossy(&out.stderr).trim()
                    ));
                }
                std::io::stdout()
                    .write_all(&out.stdout)
                    .map_err(|e| e.to_string())?;
                sent += 1;
            }
            eprintln!("fh-fakeagent: sent {sent} hook events");
            Ok(ExitCode::SUCCESS)
        }
        other => Err(format!("unknown command {other:?}")),
    }
}

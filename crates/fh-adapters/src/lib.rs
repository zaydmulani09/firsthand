//! Agent adapters: Claude Code, Codex, Gemini CLI, OpenCode, generic.
//!
//! An adapter knows one agent's on-disk formats and nothing else. It can:
//! detect the agent (`--version` only, never a prompt), find session logs whose
//! working directory is inside a repo, ingest them read-only as hearsay, plan
//! hook and MCP config edits (pure: the plan is data, [`apply_plan`] writes it),
//! translate harness hook JSON into [`fh_core::HookEvent`], and describe how to
//! launch the agent (never executed here).
//!
//! Injected hook commands are `<firsthand_cmd> hook <agent-id> <EventName>`. They
//! always contain the literal marker `firsthand hook`, which is how
//! [`uninstall_plan`] finds exactly the entries Firsthand added.

mod claude;
mod codex;
mod gemini;
mod generic;
mod hookparse;
mod ingest;
mod jsonedit;
mod opencode;
mod plan;
mod util;

pub use claude::{ClaudeCode, encode_project_dir};
pub use codex::Codex;
pub use gemini::GeminiCli;
pub use generic::Generic;
pub use opencode::{OPENCODE_PLUGIN_PATH, OpenCode, plugin_source};
pub use plan::{apply_plan, unified_diff, uninstall_plan, uninstall_plan_with};

use fh_core::{AgentId, HookEvent, HookEventKind, IngestedSession, Redactor, Result, Ts};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

/// The host environment an adapter looks at: home directory, `PATH`, and the
/// environment variables that move agent config dirs (`CLAUDE_CONFIG_DIR`,
/// `CODEX_HOME`, `XDG_DATA_HOME`, `LOCALAPPDATA`, `APPDATA`, `PATHEXT`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostEnv {
    /// Home directory.
    pub home: Option<PathBuf>,
    /// Value of `PATH`.
    pub path_var: Option<OsString>,
    /// Environment variables.
    pub env: BTreeMap<String, String>,
}

impl HostEnv {
    /// Read the current process environment.
    pub fn from_process() -> Self {
        HostEnv {
            home: directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()),
            path_var: std::env::var_os("PATH"),
            env: std::env::vars().collect(),
        }
    }

    /// A host with only a home directory (tests, fake homes).
    pub fn with_home(home: impl Into<PathBuf>) -> Self {
        HostEnv {
            home: Some(home.into()),
            ..HostEnv::default()
        }
    }

    pub(crate) fn var(&self, key: &str) -> Option<&str> {
        self.env
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }
}

/// Result of [`AgentAdapter::detect`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Detection {
    /// Whether the agent's executable was found and answered `--version`.
    pub installed: bool,
    /// First line of `--version` output.
    pub version: Option<String>,
    /// Existing config files and directories of this agent.
    pub config_paths: Vec<PathBuf>,
    /// Plain notes (what was checked, what failed).
    pub notes: Vec<String>,
}

/// The repo or worktree an operation targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoCtx {
    /// Worktree root.
    pub root: PathBuf,
    /// `git rev-parse --git-common-dir`.
    pub common_dir: PathBuf,
    /// How hooks invoke Firsthand: `firsthand`, or a quoted absolute path to the binary.
    pub firsthand_cmd: String,
}

/// Where a config edit lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookScope {
    /// Repo-local files (default).
    Repo,
    /// The user's agent config in the home directory. Only with an explicit scope.
    User,
}

/// A session log found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRef {
    /// Agent that wrote it.
    pub agent: AgentId,
    /// Session id.
    pub id: String,
    /// Session file (or the database holding it).
    pub path: PathBuf,
    /// Last modification time.
    pub modified: Option<Ts>,
    /// Working directory recorded for the session.
    pub cwd: Option<String>,
}

/// Limits for [`AgentAdapter::ingest`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngestOpts {
    /// Bytes read from a session source at most. Larger logs are read from the end.
    pub max_bytes: u64,
    /// Messages kept at most (the latest ones). Tool events use the same cap.
    pub max_messages: u32,
}

impl Default for IngestOpts {
    fn default() -> Self {
        IngestOpts {
            max_bytes: 64 * 1024 * 1024,
            max_messages: 2000,
        }
    }
}

/// One file change of a plan. `before` is the file content the plan was computed
/// against (`None`: the file does not exist). An empty `after` with `before` set
/// means the file is removed (Firsthand never writes empty config files).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEdit {
    /// File path.
    pub path: PathBuf,
    /// Content the plan expects on disk.
    pub before: Option<Vec<u8>>,
    /// Content after applying.
    pub after: Vec<u8>,
    /// Repo-local or user-level file.
    pub scope: HookScope,
    /// What the edit does, in one line.
    pub description: String,
}

impl FileEdit {
    /// Whether applying this edit deletes the file.
    pub fn is_removal(&self) -> bool {
        self.after.is_empty() && self.before.is_some()
    }
}

/// File edits plus steps a human must do (consent dialogs, trust prompts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookPlan {
    /// Agent the plan is for.
    pub agent: AgentId,
    /// File edits. Empty when everything is already in place.
    pub edits: Vec<FileEdit>,
    /// Steps Firsthand cannot do for the user.
    pub manual_steps: Vec<String>,
}

/// MCP config plans have the same shape as hook plans.
pub type ConfigPlan = HookPlan;

/// How to start an agent with a bootstrap prompt. Adapters never run it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    /// Program and arguments.
    pub argv: Vec<String>,
    /// Extra environment.
    pub env: Vec<(String, String)>,
    /// Working directory.
    pub cwd: PathBuf,
    /// Whether this command line was checked against a live install.
    pub verified: bool,
}

/// The brief a launch should point the agent at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BriefRef {
    /// Packet id.
    pub packet_id: String,
    /// Brief path relative to the worktree root.
    pub brief_path: String,
    /// Bootstrap prompt (spec 11.4).
    pub bootstrap: String,
}

/// How completely the harness reports an activity to hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Witness {
    /// Every occurrence reaches a hook, with its result.
    Full,
    /// Some occurrences or fields are missing, or a manual step gates it.
    Partial,
    /// Not observable through hooks.
    None,
}

/// How a bootstrap prompt reaches a new session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Inject {
    /// SessionStart hook stdout is added to the agent's context.
    Stdout,
    /// Through a file the agent reads (AGENTS.md managed block).
    File,
    /// No automatic delivery.
    None,
}

/// What an adapter can observe and do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Shell commands and their results.
    pub witness_bash: Witness,
    /// File edits by the agent's edit tools.
    pub witness_edits: Witness,
    /// Pre-tool events exist.
    pub pre_tool_events: bool,
    /// Bootstrap delivery at session start.
    pub session_start_inject: Inject,
    /// A stop (end of turn) event exists.
    pub stop_event: bool,
    /// A compaction event exists.
    pub compact_event: bool,
    /// Subagent events exist.
    pub subagent_events: bool,
    /// Session logs can be ingested.
    pub ingest: bool,
    /// MCP config can be generated.
    pub mcp: bool,
}

/// One agent's formats and config conventions (spec 10.1).
pub trait AgentAdapter: Send + Sync {
    /// Agent id.
    fn id(&self) -> AgentId;
    /// Installed?, version, config paths. Runs `<agent> --version` and nothing else.
    fn detect(&self, host: &HostEnv) -> Detection;
    /// What this adapter can observe.
    fn capabilities(&self) -> Capabilities;
    /// Sessions whose working directory is inside the repo or worktree, newest first.
    fn discover_sessions(&self, repo: &RepoCtx) -> Result<Vec<SessionRef>>;
    /// Read a session log. Read-only; the output is hearsay and unredacted (see [`redact_session`]).
    fn ingest(&self, s: &SessionRef, opts: &IngestOpts) -> Result<IngestedSession>;
    /// Hook config edits. Pure: reads current files, writes nothing.
    fn hook_plan(&self, repo: &RepoCtx, scope: HookScope) -> Result<HookPlan>;
    /// Harness hook JSON (stdin, or the last argv for Codex `notify`) to a normalized event.
    /// `argv_tail` is everything after `firsthand hook <agent>`, starting with the event name.
    fn parse_hook_event(&self, raw: &[u8], argv_tail: &[String]) -> Result<HookEvent>;
    /// MCP server config edits. Pure.
    fn mcp_install_plan(&self, repo: &RepoCtx, scope: HookScope) -> Result<ConfigPlan>;
    /// Command line that starts the agent on a brief. Never executed by the adapter.
    fn launch_spec(&self, brief: &BriefRef, repo: &RepoCtx) -> Result<LaunchSpec>;
}

/// Every adapter, in display order, looking at the current process environment.
pub fn all() -> Vec<Box<dyn AgentAdapter>> {
    AgentId::ALL.iter().map(|a| get(*a)).collect()
}

/// The adapter for one agent, looking at the current process environment.
pub fn get(id: AgentId) -> Box<dyn AgentAdapter> {
    get_with(id, HostEnv::from_process())
}

/// The adapter for one agent with an explicit host (fake homes in tests).
pub fn get_with(id: AgentId, host: HostEnv) -> Box<dyn AgentAdapter> {
    match id {
        AgentId::ClaudeCode => Box::new(ClaudeCode::new(host)),
        AgentId::Codex => Box::new(Codex::new(host)),
        AgentId::GeminiCli => Box::new(GeminiCli::new(host)),
        AgentId::Opencode => Box::new(OpenCode::new(host)),
        AgentId::Generic => Box::new(Generic),
    }
}

/// Whether a hook command was injected by Firsthand. Matches `firsthand hook`,
/// `firsthand.exe hook`, and quoted paths ending in `firsthand"` or `firsthand.exe"`
/// followed by ` hook`. Case-insensitive.
pub fn is_firsthand_hook_command(cmd: &str) -> bool {
    let lower = cmd.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(" hook") {
        let at = from + i;
        let after = &lower[at + 5..];
        let before = lower[..at].trim_end_matches(['"', '\'']);
        let stem = before
            .strip_suffix(".exe")
            .unwrap_or(before)
            .strip_suffix("firsthand");
        let boundary =
            stem.is_some_and(|s| s.is_empty() || s.ends_with(['/', '\\', '"', '\'', ' ']));
        if (after.is_empty() || after.starts_with(' ')) && boundary {
            return true;
        }
        from = at + 5;
    }
    false
}

/// Text a `firsthand hook` process prints for an event. Most harnesses add plain
/// SessionStart stdout to the context; Gemini CLI parses stdout as JSON, so the text
/// goes into `hookSpecificOutput.additionalContext`.
pub fn hook_stdout(agent: AgentId, kind: HookEventKind, text: &str) -> String {
    if agent == AgentId::GeminiCli && kind == HookEventKind::SessionStart {
        serde_json::json!({
            "hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": text}
        })
        .to_string()
    } else {
        text.to_owned()
    }
}

/// Redact every text field of an ingested session in place. Adapters return raw
/// text because they do not own redaction rules; callers run this before anything
/// is stored.
pub fn redact_session(s: &mut IngestedSession, r: &dyn Redactor) {
    let fix = |t: &mut String| *t = r.redact(t).text;
    for m in &mut s.messages {
        fix(&mut m.text);
    }
    for t in &mut s.tool_events {
        fix(&mut t.tool);
        fix(&mut t.summary);
        if let Some(o) = &mut t.output {
            fix(o);
        }
    }
    fix(&mut s.final_assistant_text);
    for o in [&mut s.cwd, &mut s.git_branch].into_iter().flatten() {
        fix(o);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_matching() {
        assert!(is_firsthand_hook_command(
            "firsthand hook claude-code PreToolUse"
        ));
        assert!(is_firsthand_hook_command(
            "\"C:\\Program Files\\firsthand\\firsthand.exe\" hook codex Stop"
        ));
        assert!(is_firsthand_hook_command("/usr/local/bin/firsthand hook x"));
        assert!(is_firsthand_hook_command("FIRSTHAND.EXE hook"));
        assert!(!is_firsthand_hook_command("firsthand hooks"));
        assert!(!is_firsthand_hook_command("echo hook"));
        assert!(!is_firsthand_hook_command("myfirsthandx hook"));
        assert!(!is_firsthand_hook_command("notfirsthand hook"));
        assert!(!is_firsthand_hook_command("firsthand mcp serve"));
    }

    #[test]
    fn gemini_session_start_is_json() {
        let s = hook_stdout(AgentId::GeminiCli, HookEventKind::SessionStart, "read x");
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["hookSpecificOutput"]["additionalContext"], "read x");
        assert_eq!(
            hook_stdout(AgentId::ClaudeCode, HookEventKind::SessionStart, "read x"),
            "read x"
        );
    }

    #[test]
    fn registry_covers_every_agent() {
        let ids: Vec<AgentId> = all().iter().map(|a| a.id()).collect();
        assert_eq!(ids, AgentId::ALL.to_vec());
    }
}

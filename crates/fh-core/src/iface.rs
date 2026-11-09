//! Cross-crate interface types, frozen in Phase 0 so parallel tracks compile
//! against the same shapes: normalized hook events, ingested sessions, drift
//! reports, claim evaluations, and the redaction trait.

use crate::B3;
use crate::hash::GitOid;
use crate::id::{Clm, Id};
use crate::model::{AgentId, ClaimStatus, FileOp, LedgerRef, ProbeResult, SymbolKind, Tier};
use crate::time::Ts;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ---------------------------------------------------------------- hooks

/// Normalized harness hook event, produced by an adapter's `parse_hook_event`
/// and consumed by the ledger's hook fast path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HookEvent {
    /// Agent that emitted the event.
    pub agent: AgentId,
    /// Normalized event kind.
    pub kind: HookEventKind,
    /// The harness's own event name, as received.
    pub raw_event_name: String,
    /// Agent session id.
    pub session_id: Option<String>,
    /// Tool use id for pairing Pre and Post events.
    pub tool_use_id: Option<String>,
    /// Working directory reported by the harness. Used for repo discovery only; never stored.
    pub cwd: Option<String>,
    /// Transcript path reported by the harness. Used for ingestion only; never stored.
    pub transcript_path: Option<String>,
    /// SessionStart source (`startup`, `resume`, `clear`, `compact`).
    pub source: Option<String>,
    /// Tool call details for tool events.
    pub tool: Option<ToolCall>,
    /// Agent version, if reported.
    pub agent_version: Option<String>,
    /// Model, if reported.
    pub model: Option<String>,
}

/// Normalized hook event kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HookEventKind {
    /// Session started or resumed.
    SessionStart,
    /// Before a tool runs.
    PreToolUse,
    /// After a tool ran.
    PostToolUse,
    /// The agent stopped (end of a turn).
    Stop,
    /// A subagent stopped.
    SubagentStop,
    /// Context is about to be compacted.
    PreCompact,
    /// Session ended.
    SessionEnd,
    /// The user submitted a prompt.
    UserPromptSubmit,
    /// Anything else.
    Other,
}

/// A tool call inside a hook event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    /// Tool name as the harness reports it (`Bash`, `Write`, `apply_patch`).
    pub name: String,
    /// Classification.
    pub class: ToolClass,
    /// Shell command text, for shell tools.
    pub command: Option<String>,
    /// argv, when the harness provides one.
    pub argv: Option<Vec<String>>,
    /// Edited file paths (as reported; normalized by the ledger), for edit tools.
    pub file_paths: Vec<String>,
    /// File operation, for edit tools.
    pub file_op: Option<FileOp>,
    /// Exit code, for completed shell tools.
    pub exit_code: Option<i32>,
    /// stdout, for completed shell tools (unredacted here; the ledger redacts).
    pub stdout: Option<String>,
    /// stderr, for completed shell tools.
    pub stderr: Option<String>,
    /// Whether the harness marked the result as an error.
    pub is_error: Option<bool>,
    /// Short summary for other tools.
    pub summary: Option<String>,
}

/// Tool classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolClass {
    /// Runs shell commands.
    Shell,
    /// Writes or edits files.
    FileEdit,
    /// Anything else.
    Other,
}

/// What the hook process should print, if anything.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HookOutcome {
    /// Text for stdout (SessionStart bootstrap injection).
    pub stdout: Option<String>,
}

// ---------------------------------------------------------------- ingestion

/// A session log read by an adapter. Everything in it is hearsay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IngestedSession {
    /// Source agent.
    pub agent: AgentId,
    /// Session id.
    pub session_id: String,
    /// Working directory recorded in the log (absolute; never written to packets).
    pub cwd: Option<String>,
    /// Branch recorded in the log.
    pub git_branch: Option<String>,
    /// Start time.
    pub started: Option<Ts>,
    /// End time.
    pub ended: Option<Ts>,
    /// Messages in order. Hidden reasoning is never included.
    pub messages: Vec<IngestedMessage>,
    /// Tool events in order.
    pub tool_events: Vec<IngestedToolEvent>,
    /// Text of the final assistant turn.
    pub final_assistant_text: String,
    /// BLAKE3 of the source bytes.
    pub source_hash: B3,
    /// Lines or records skipped as malformed.
    pub skipped_records: u32,
}

/// One message of an ingested session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IngestedMessage {
    /// Role.
    pub role: Role,
    /// Text content.
    pub text: String,
    /// Timestamp.
    pub ts: Option<Ts>,
    /// Whether the harness marked it as a summary (compaction).
    pub is_summary: bool,
}

/// Message role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The user.
    User,
    /// The assistant.
    Assistant,
    /// System or harness text.
    System,
    /// Tool output.
    Tool,
}

/// One tool event of an ingested session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IngestedToolEvent {
    /// Tool name.
    pub tool: String,
    /// Short summary (command text, file path).
    pub summary: String,
    /// Output excerpt.
    pub output: Option<String>,
    /// Timestamp.
    pub ts: Option<Ts>,
    /// Whether it errored.
    pub is_error: bool,
}

// ---------------------------------------------------------------- drift

/// Relation of the current HEAD to an anchor commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum HeadRelation {
    /// Same commit.
    Same,
    /// HEAD is `n` commits ahead of the anchor.
    Ahead {
        /// Commits ahead.
        n: u32,
    },
    /// HEAD is `n` commits behind the anchor.
    Behind {
        /// Commits behind.
        n: u32,
    },
    /// Both sides have commits.
    Diverged {
        /// Commits only in HEAD.
        ahead: u32,
        /// Commits only in the anchor.
        behind: u32,
    },
    /// No common history.
    Unrelated,
    /// The anchor commit is not in this clone.
    AnchorMissing,
}

/// One entry of `git diff --name-status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PathChange {
    /// `A`, `M`, `D`, `R`, `C`, `T`.
    pub status: String,
    /// Path (new path for renames).
    pub path: String,
    /// Old path for renames and copies.
    pub old_path: Option<String>,
}

/// Kind of symbol change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SymbolChangeKind {
    /// New symbol.
    Added,
    /// Symbol removed.
    Removed,
    /// Body changed.
    BodyChanged,
    /// Signature changed.
    SigChanged,
    /// Moved to another file with identical hashes.
    Moved,
}

/// A symbol-level change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SymbolChange {
    /// File (new location for moves).
    pub path: String,
    /// Name.
    pub name: String,
    /// Kind.
    pub kind: SymbolKind,
    /// Change.
    pub change: SymbolChangeKind,
    /// Previous file, for moves.
    pub old_path: Option<String>,
}

/// Why a claim is affected by drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Cause {
    /// A scoped file changed.
    FileChanged {
        /// Path.
        path: String,
        /// Blob at anchor.
        before: Option<GitOid>,
        /// Blob now.
        after: Option<GitOid>,
    },
    /// A scoped symbol changed.
    SymbolChanged {
        /// File.
        path: String,
        /// Name.
        name: String,
        /// Change kind.
        change: SymbolChangeKind,
    },
    /// The repo tree changed (repo-scoped claims).
    TreeChanged {
        /// Tree at anchor.
        before: GitOid,
        /// Tree now.
        after: GitOid,
        /// Up to 20 changed paths, for the explanation.
        sample_paths: Vec<String>,
    },
    /// The branch differs from the anchor.
    BranchChanged {
        /// Branch at anchor.
        before: Option<String>,
        /// Branch now.
        after: Option<String>,
    },
    /// The anchor commit is missing locally.
    AnchorMissing,
}

/// A claim affected by drift and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimAffected {
    /// Claim id.
    pub claim_id: Id<Clm>,
    /// Causes.
    pub causes: Vec<Cause>,
}

/// Difference between a packet's anchor and the receiver's current tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DriftReport {
    /// HEAD relation.
    pub head_relation: HeadRelation,
    /// Commits in HEAD not in the anchor.
    pub commits_since_anchor: u32,
    /// Files changed between the anchor snapshot tree and the current working tree.
    pub files_changed: Vec<PathChange>,
    /// Symbols changed in those files (supported languages only).
    pub symbols_changed: Vec<SymbolChange>,
    /// Claims affected, with causes.
    pub claims_affected: Vec<ClaimAffected>,
    /// Whether the branch differs from the anchor.
    pub branch_changed: bool,
    /// Whether the worktree has uncommitted changes now.
    pub worktree_dirty_now: bool,
    /// Current working tree oid (snapshot tree of the receiver state).
    pub current_tree: GitOid,
    /// What could not be computed, in plain words.
    pub notes: Vec<String>,
}

// ---------------------------------------------------------------- evaluation

/// The evaluated state of one claim against the receiver's tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimEval {
    /// Claim id.
    pub claim_id: Id<Clm>,
    /// Computed status.
    pub status: ClaimStatus,
    /// Trust tier of the evidence behind the status.
    pub tier: Tier,
    /// Drift causes (stale) or the contradiction (refuted).
    pub causes: Vec<Cause>,
    /// Ledger entry cited as evidence (refuted or witnessed).
    pub evidence: Option<LedgerRef>,
    /// Probe result produced by this evaluation, if a probe ran.
    pub result: Option<ProbeResult>,
    /// One-line explanation shown to humans.
    pub detail: String,
    /// For unverified claims: the probe that would settle it, in words.
    pub suggested_probe: Option<String>,
    /// For commands that need approval: the exact argv.
    pub needs_approval: Option<Vec<String>>,
}

// ---------------------------------------------------------------- redaction

/// Result of redacting a text.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Redacted {
    /// Redacted text.
    pub text: String,
    /// Replacements per rule id.
    pub findings: BTreeMap<String, u32>,
}

impl Redacted {
    /// Total replacements.
    pub fn count(&self) -> u32 {
        self.findings.values().sum()
    }
}

/// Redacts secrets and identity from text. Implemented by `fh-redact`.
/// Must be idempotent: redacting redacted text changes nothing.
pub trait Redactor: Send + Sync {
    /// Redact a text.
    fn redact(&self, text: &str) -> Redacted;
}

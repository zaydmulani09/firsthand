//! The frozen data model (master spec Section 5 and 6.3). Field names are final.
//! JSON uses snake_case. Every struct rejects unknown fields; extension data
//! lives only under `extensions` maps whose keys start with `x-`.

use crate::hash::{B3, GitOid};
use crate::id::{Att, Clm, Dec, Id, Pkt, Snp, Tsk};
use crate::time::Ts;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

macro_rules! unit_enum {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $v:ident),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($(#[$vm])* $v),* }
    };
}

unit_enum! {
    /// Trust tier of a piece of evidence.
    Tier {
        /// Evaluated by the receiving Firsthand on the receiver's tree.
        Replayed,
        /// Observed by a Firsthand entry point during the session.
        Witnessed,
        /// Asserted with no passing probe. Shown as HEARSAY.
        Reported,
    }
}

unit_enum! {
    /// Computed status of a claim.
    ClaimStatus {
        /// Probe passed on the receiver's current tree.
        Verified,
        /// Witnessed pass and the scope is byte-identical now.
        Holds,
        /// Scope changed since the witnessed pass and no rerun happened.
        Stale,
        /// A probe failed now, or witnessed evidence contradicts the claim.
        Refuted,
        /// No probe could be attached.
        Unverified,
        /// Probe errored, timed out, or needs approval.
        Unknown,
    }
}

unit_enum! {
    /// How an event reached the ledger. Assigned by entry point code, never read from input.
    Channel {
        /// Harness hook event.
        Hook,
        /// `firsthand run`.
        Run,
        /// MCP tool call.
        Mcp,
        /// `firsthand note`, `claim`, `task`.
        Cli,
        /// Parsed from a session log.
        Ingest,
        /// Parsed from a markdown handoff.
        Import,
    }
}

/// Supported agents.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum AgentId {
    /// Claude Code.
    ClaudeCode,
    /// OpenAI Codex CLI.
    Codex,
    /// Gemini CLI.
    GeminiCli,
    /// OpenCode.
    Opencode,
    /// Any other agent, through `firsthand run`, MCP, or the CLI.
    Generic,
}

impl AgentId {
    /// All agents in display order.
    pub const ALL: [AgentId; 5] = [
        AgentId::ClaudeCode,
        AgentId::Codex,
        AgentId::GeminiCli,
        AgentId::Opencode,
        AgentId::Generic,
    ];

    /// The kebab-case id string.
    pub fn as_str(self) -> &'static str {
        match self {
            AgentId::ClaudeCode => "claude-code",
            AgentId::Codex => "codex",
            AgentId::GeminiCli => "gemini-cli",
            AgentId::Opencode => "opencode",
            AgentId::Generic => "generic",
        }
    }

    /// Parse an id or a common short alias (`claude`, `gemini`).
    pub fn parse(s: &str) -> Option<AgentId> {
        match s.to_ascii_lowercase().as_str() {
            "claude-code" | "claude" | "claudecode" => Some(AgentId::ClaudeCode),
            "codex" => Some(AgentId::Codex),
            "gemini-cli" | "gemini" => Some(AgentId::GeminiCli),
            "opencode" => Some(AgentId::Opencode),
            "generic" => Some(AgentId::Generic),
            _ => None,
        }
    }
}

unit_enum! {
    /// Git object format.
    ObjectFormat {
        /// SHA-1, 40 hex digits.
        Sha1,
        /// SHA-256, 64 hex digits.
        Sha256,
    }
}

// ---------------------------------------------------------------- anchor, snapshot

/// The repo identity, HEAD, branch and snapshot a packet is true at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepoAnchor {
    /// BLAKE3 of the newline-joined sorted root commit oids. Stable across clones.
    pub repo_id: B3,
    /// Object format of the repository.
    pub object_format: ObjectFormat,
    /// HEAD commit, `None` on an unborn branch.
    pub head: Option<GitOid>,
    /// Branch name, `None` if detached.
    pub branch: Option<String>,
    /// Upstream tracking ref, if any.
    pub upstream: Option<String>,
    /// Snapshot of the full working state.
    pub snapshot: SnapshotRef,
    /// Worktree information, without absolute paths.
    pub worktree: WorktreeInfo,
    /// Summary of uncommitted changes.
    pub dirty: DirtySummary,
    /// `git --version` output.
    pub git_version: String,
}

/// A snapshot commit kept alive by a private ref.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SnapshotRef {
    /// Snapshot id.
    pub id: Id<Snp>,
    /// Snapshot commit.
    pub commit: GitOid,
    /// Tree of the snapshot commit.
    pub tree: GitOid,
    /// Parent commit (HEAD at snapshot time), if any.
    pub base: Option<GitOid>,
    /// Ref that keeps the commit alive, e.g. `refs/firsthand/snapshots/snp_...`.
    pub ref_name: String,
    /// Why the snapshot was taken.
    pub kind: SnapshotKind,
    /// Creation time.
    pub created_at: Ts,
}

unit_enum! {
    /// Why a snapshot was taken.
    SnapshotKind {
        /// `firsthand snapshot`.
        Manual,
        /// Rotating automatic snapshot (Stop hook).
        Auto,
        /// Taken while sealing a packet.
        Handoff,
        /// Taken before a context compaction.
        PreCompact,
        /// Taken before `restore --apply`.
        PreRestore,
    }
}

/// Linked worktree information.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorktreeInfo {
    /// Whether this is a linked worktree.
    pub linked: bool,
    /// Worktree name (directory name under `.git/worktrees`), if linked.
    pub name: Option<String>,
}

/// Counts of uncommitted changes and files deliberately left out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DirtySummary {
    /// Tracked files modified.
    pub modified: u32,
    /// Files added to the index or new.
    pub added: u32,
    /// Tracked files deleted.
    pub deleted: u32,
    /// Untracked, non-ignored files.
    pub untracked: u32,
    /// Files that would have been captured but were excluded.
    pub omitted: Vec<Omitted>,
}

/// A file excluded from a snapshot, with the reason. Nothing is dropped silently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Omitted {
    /// Repo-relative path with `/` separators.
    pub path: String,
    /// Why it was excluded.
    pub reason: OmitReason,
    /// Size in bytes, if known.
    pub bytes: Option<u64>,
    /// BLAKE3 of the content, if computed (never for secrets).
    pub hash: Option<B3>,
}

unit_enum! {
    /// Reason a file was excluded from a snapshot.
    OmitReason {
        /// Path matched a secret glob.
        SecretPattern,
        /// Content matched a secret rule.
        SecretContent,
        /// Larger than `snapshot.max_file_bytes`.
        TooLarge,
        /// Submodule content (recorded as a gitlink only).
        Submodule,
    }
}

// ---------------------------------------------------------------- legs, ledger

/// One contiguous stretch of work by one agent session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Leg {
    /// Leg id.
    pub id: Id<Leg>,
    /// The agent.
    pub agent: AgentRef,
    /// The agent's own session id, if known.
    pub agent_session: Option<String>,
    /// Start time.
    pub started_at: Ts,
    /// End time.
    pub ended_at: Option<Ts>,
    /// Snapshot at leg start.
    pub anchor_start: Option<SnapshotRef>,
    /// Snapshot at leg end.
    pub anchor_end: Option<SnapshotRef>,
    /// Packet this leg continued from.
    pub parent_packet: Option<PacketRef>,
    /// How the leg ended.
    pub outcome: LegOutcome,
    /// Ledger range covered by this leg.
    pub ledger: LedgerRange,
}

/// Agent identity and version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentRef {
    /// Agent id.
    pub id: AgentId,
    /// Agent version, from `--version` or hook payloads.
    pub version: Option<String>,
    /// Model name, if the harness reports it.
    pub model: Option<String>,
}

unit_enum! {
    /// How a leg ended.
    LegOutcome {
        /// Still running.
        Open,
        /// Sealed into a packet.
        HandedOff,
        /// Finished the work.
        Completed,
        /// Ended without a handoff.
        Abandoned,
        /// Context compacted.
        Compacted,
    }
}

/// Range of a leg's ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LedgerRange {
    /// First sequence number.
    pub first_seq: u64,
    /// Last sequence number.
    pub last_seq: u64,
    /// Hash of the last entry.
    pub head_hash: B3,
}

/// Reference to a packet by id and manifest hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PacketRef {
    /// Packet id.
    pub id: Id<Pkt>,
    /// BLAKE3 of the canonical manifest.
    pub manifest_hash: B3,
}

/// Reference to one ledger entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LedgerRef {
    /// Leg id.
    pub leg: Id<Leg>,
    /// Sequence number within the leg.
    pub seq: u64,
    /// Hash of the entry.
    pub hash: B3,
}

impl LedgerRef {
    /// Event id string `<leg_id>#<seq>`.
    pub fn evt(&self) -> String {
        format!("{}#{}", self.leg, self.seq)
    }
}

/// One hash-chained ledger line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LedgerEntry {
    /// Format version, always 1.
    pub v: u8,
    /// Sequence number, starting at 0 per leg.
    pub seq: u64,
    /// Hash of the previous entry; for seq 0, `blake3("firsthand.ledger/1" || leg_id)`.
    pub prev: B3,
    /// Event time.
    pub ts: Ts,
    /// Leg id.
    pub leg: Id<Leg>,
    /// Entry kind.
    pub kind: EntryKind,
    /// How the entry reached the ledger.
    pub witness: Witness,
    /// Kind-specific body, integers only.
    pub body: serde_json::Value,
    /// `blake3(canonical(entry without hash))`.
    pub hash: B3,
}

/// Witness metadata of a ledger entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Witness {
    /// Channel, set by the entry point.
    pub channel: Channel,
    /// Agent that produced the event.
    pub agent: AgentId,
    /// Agent session id.
    pub agent_session: Option<String>,
    /// Harness tool use id, for pairing Pre and Post events.
    pub tool_use_id: Option<String>,
}

unit_enum! {
    /// Kind of ledger entry.
    EntryKind {
        /// Leg started.
        LegStart,
        /// A command ran.
        Command,
        /// A file was written, edited, deleted or renamed by a tool.
        FileEdit,
        /// A parsed test result.
        TestResult,
        /// A snapshot was taken.
        Snapshot,
        /// Free text note.
        Note,
        /// Claim create/update/verify.
        Claim,
        /// Decision op.
        Decision,
        /// Task op.
        Task,
        /// Attempt op.
        Attempt,
        /// A session log was ingested.
        Ingest,
        /// A brief was delivered to an agent session.
        BriefDelivered,
        /// Another tool event, summarized.
        ToolOther,
        /// Leg ended.
        LegEnd,
    }
}

/// Reference to a stored output blob.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlobRef {
    /// Hash of the stored (redacted, possibly truncated) bytes.
    pub hash: B3,
    /// Original size.
    pub bytes: u64,
    /// Stored size.
    pub stored_bytes: u64,
    /// Whether the middle was elided.
    pub truncated: bool,
    /// Whether redaction replaced anything.
    pub redacted: bool,
}

unit_enum! {
    /// Kind of verification command.
    VerificationKind {
        /// Test runner.
        Test,
        /// Build.
        Build,
        /// Linter.
        Lint,
        /// Type checker.
        Typecheck,
        /// Recognized but uncategorized.
        Other,
    }
}

/// Parsed test counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TestSummary {
    /// Runner name (`cargo`, `pytest`, `vitest`, `jest`, `go`).
    pub runner: String,
    /// Passed tests.
    pub passed: u64,
    /// Failed tests.
    pub failed: u64,
    /// Skipped or ignored tests.
    pub skipped: u64,
}

/// Body of a `command` entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandBody {
    /// argv, when known.
    pub argv: Option<Vec<String>>,
    /// Command as text (redacted).
    pub command_text: String,
    /// Working directory relative to the repo root (`.` for root).
    pub cwd_rel: String,
    /// Exit code, if it finished.
    pub exit_code: Option<i32>,
    /// Start time.
    pub started_at: Ts,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Captured stdout.
    pub stdout: BlobRef,
    /// Captured stderr.
    pub stderr: BlobRef,
    /// Tree before the command (verification commands only).
    pub tree_before: Option<GitOid>,
    /// Tree after the command (verification commands only).
    pub tree_after: Option<GitOid>,
    /// Cheap fingerprint of the dirty state.
    pub dirty_fp: Option<B3>,
    /// Who ran it.
    pub runner: CommandRunner,
    /// Verification kind, if the command matched a pattern.
    pub verification_kind: Option<VerificationKind>,
    /// Parsed test counts.
    pub test_summary: Option<TestSummary>,
}

/// Who executed a witnessed command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum CommandRunner {
    /// The agent's shell tool, observed by a hook.
    #[serde(rename = "bash-tool")]
    BashTool,
    /// `firsthand run`.
    #[serde(rename = "fh-run")]
    FhRun,
}

/// Body of a `file_edit` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileEditBody {
    /// Repo-relative path.
    pub path: String,
    /// Operation.
    pub op: FileOp,
    /// Blob before.
    pub before_blob: Option<GitOid>,
    /// Blob after.
    pub after_blob: Option<GitOid>,
    /// Tool name reported by the harness.
    pub tool: String,
}

unit_enum! {
    /// File operation.
    FileOp {
        /// Whole-file write.
        Write,
        /// In-place edit.
        Edit,
        /// Deletion.
        Delete,
        /// Rename.
        Rename,
    }
}

/// Body of an `ingest` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IngestBody {
    /// Source agent.
    pub agent: AgentId,
    /// Session id.
    pub session_id: String,
    /// Hash of the source file bytes.
    pub source_hash: B3,
    /// Messages read.
    pub messages: u64,
    /// Tool events read.
    pub tool_events: u64,
    /// Claims extracted.
    pub claims_extracted: u64,
}

/// Body of a `brief_delivered` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BriefDeliveredBody {
    /// Packet delivered.
    pub packet_id: Id<Pkt>,
    /// Receiving agent.
    pub agent: AgentId,
    /// Receiving session.
    pub session_id: Option<String>,
}

/// Body of a `tool_other` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolOtherBody {
    /// Tool name.
    pub tool_name: String,
    /// Truncated, redacted summary.
    pub summary: String,
}

/// Body of a `note` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoteBody {
    /// Note text (redacted).
    pub text: String,
}

/// Body of a `snapshot` entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SnapshotBody {
    /// The snapshot.
    pub snapshot: SnapshotRef,
}

/// Body of `claim`, `decision`, `task`, `attempt` entries: an op plus a full object or patch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpBody {
    /// Operation name.
    pub op: Op,
    /// Target id, for ops on an existing object.
    pub id: Option<String>,
    /// Full object (create, supersede) or patch (update).
    pub data: serde_json::Value,
}

unit_enum! {
    /// Mutation operation on claims, tasks, decisions and attempts.
    Op {
        /// Create a new object.
        Create,
        /// Patch fields.
        Update,
        /// Record a probe result.
        Verify,
        /// Replace with a new object.
        Supersede,
        /// Mark a task done.
        Done,
        /// Mark a task blocked.
        Block,
        /// Drop a task.
        Drop,
    }
}

// ---------------------------------------------------------------- claims

/// A statement about the repo or the work, with scope, probe and results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    /// Claim id.
    pub id: Id<Clm>,
    /// Claim text (agent-authored text is untrusted).
    pub text: String,
    /// Category.
    pub category: ClaimCategory,
    /// Who asserted it.
    pub origin: ClaimOrigin,
    /// What it depends on.
    pub scope: Scope,
    /// How to check it.
    pub probe: Option<Probe>,
    /// Probe results so far.
    pub results: Vec<ProbeResult>,
    /// Creation time.
    pub created_at: Ts,
    /// Leg that created it.
    pub leg: Id<Leg>,
    /// Ledger entry that created it.
    pub ledger_ref: LedgerRef,
}

unit_enum! {
    /// Claim category.
    ClaimCategory {
        /// A witnessed verification command succeeded.
        CommandSucceeded,
        /// A witnessed verification command failed.
        CommandFailed,
        /// Tests pass.
        TestsPass,
        /// Build succeeds.
        BuildOk,
        /// Lint is clean.
        LintOk,
        /// A file was created.
        FileCreated,
        /// A file was modified.
        FileModified,
        /// A file was deleted.
        FileDeleted,
        /// A symbol was added.
        SymbolAdded,
        /// A refactor is complete.
        RefactorDone,
        /// A bug was fixed.
        BugFixed,
        /// A task is done.
        TaskDone,
        /// A statement about a branch.
        Branch,
        /// Anything else.
        Other,
    }
}

/// Who asserted a claim and how confident the source is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimOrigin {
    /// Source kind.
    pub source: OriginSource,
    /// Agent, if any.
    pub agent: Option<AgentId>,
    /// Confidence in permille (0 to 1000). Integer, never a float.
    pub confidence_permille: u16,
}

unit_enum! {
    /// Origin of a claim.
    OriginSource {
        /// Derived from witnessed events.
        Witnessed,
        /// Declared by the agent.
        AgentDeclared,
        /// Declared by a human.
        Human,
        /// Extracted from transcript narration.
        TranscriptExtracted,
        /// Imported from a markdown handoff.
        Imported,
    }
}

/// What a claim depends on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Scope {
    /// The whole tree.
    Repo {
        /// Tree oid at anchor time.
        tree: GitOid,
    },
    /// Specific files and symbols.
    Items {
        /// Files with blob oids.
        files: Vec<ScopedFile>,
        /// Symbols with body and signature hashes.
        symbols: Vec<ScopedSymbol>,
    },
}

/// A file in a claim scope.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScopedFile {
    /// Repo-relative path.
    pub path: String,
    /// Blob at anchor time; `None` means absent at anchor.
    pub blob: Option<GitOid>,
}

/// A symbol in a claim scope.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScopedSymbol {
    /// File containing the symbol at anchor time.
    pub path: String,
    /// Symbol name.
    pub name: String,
    /// Symbol kind.
    pub kind: SymbolKind,
    /// Formatting-insensitive body hash.
    pub body_hash: B3,
    /// Signature hash.
    pub sig_hash: B3,
}

unit_enum! {
    /// Kind of source symbol.
    SymbolKind {
        /// Free function.
        Fn,
        /// Method.
        Method,
        /// Class.
        Class,
        /// Struct.
        Struct,
        /// Enum.
        Enum,
        /// Trait.
        Trait,
        /// Interface.
        Interface,
        /// Type alias.
        Type,
        /// Constant.
        Const,
        /// Module or namespace.
        Module,
    }
}

unit_enum! {
    /// Comparison operator.
    CmpOp {
        /// Equal.
        Eq,
        /// Not equal.
        Ne,
        /// Less than.
        Lt,
        /// Less or equal.
        Le,
        /// Greater than.
        Gt,
        /// Greater or equal.
        Ge,
    }
}

impl CmpOp {
    /// Apply the comparison.
    pub fn eval(self, a: u64, b: u64) -> bool {
        match self {
            CmpOp::Eq => a == b,
            CmpOp::Ne => a != b,
            CmpOp::Lt => a < b,
            CmpOp::Le => a <= b,
            CmpOp::Gt => a > b,
            CmpOp::Ge => a >= b,
        }
    }
}

/// A checkable procedure attached to a claim. Everything except `Command` is static.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Probe {
    /// File exists in the tree.
    FileExists {
        /// Path.
        path: String,
    },
    /// File is absent from the tree.
    FileAbsent {
        /// Path.
        path: String,
    },
    /// File has exactly this blob.
    FileHash {
        /// Path.
        path: String,
        /// Expected blob.
        blob: GitOid,
    },
    /// File content matches a regex at least `min` times.
    FileContains {
        /// Path.
        path: String,
        /// Rust regex syntax.
        pattern: String,
        /// Minimum matches.
        min: u32,
    },
    /// Number of files matching a glob compares to `n`.
    GlobCount {
        /// Glob.
        glob: String,
        /// Comparison.
        op: CmpOp,
        /// Count.
        n: u32,
    },
    /// A commit exists.
    GitCommitExists {
        /// Commit.
        oid: GitOid,
    },
    /// A branch points at a commit.
    GitBranchAt {
        /// Branch name.
        branch: String,
        /// Commit.
        oid: GitOid,
    },
    /// A path has no uncommitted changes.
    GitPathClean {
        /// Path.
        path: String,
    },
    /// A symbol is defined.
    SymbolDefined {
        /// Restrict to a file.
        path: Option<String>,
        /// Name.
        name: String,
        /// Restrict to a kind.
        kind: Option<SymbolKind>,
    },
    /// No symbol with this name exists.
    SymbolAbsent {
        /// Name.
        name: String,
    },
    /// A symbol body has exactly this hash.
    SymbolHash {
        /// File.
        path: String,
        /// Name.
        name: String,
        /// Expected body hash.
        body_hash: B3,
    },
    /// Cross-check against the latest witnessed verification command. Executes nothing.
    LedgerVerification {
        /// Kind of verification.
        kind: VerificationKind,
        /// Optional argv prefix filter.
        matcher: Option<CommandMatcher>,
        /// What counts as passing.
        require: LedgerRequire,
    },
    /// Execute a program. Policy gated.
    Command {
        /// argv.
        argv: Vec<String>,
        /// Working directory relative to the repo root.
        cwd_rel: String,
        /// Run through a shell (requires `allow_shell` and explicit approval).
        shell: bool,
        /// Timeout in seconds.
        timeout_s: u32,
        /// Expected outcome.
        expect: Expect,
    },
    /// All sub-probes must pass.
    All {
        /// Sub-probes.
        probes: Vec<Probe>,
    },
}

impl Probe {
    /// Whether evaluating this probe executes a program.
    pub fn executes(&self) -> bool {
        match self {
            Probe::Command { .. } => true,
            Probe::All { probes } => probes.iter().any(Probe::executes),
            _ => false,
        }
    }
}

unit_enum! {
    /// Requirement for a ledger verification probe.
    LedgerRequire {
        /// Exit code 0.
        ExitZero,
        /// Parsed tests: zero failed and more than zero passed.
        TestsAllPass,
    }
}

/// Filters witnessed commands by argv prefix.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandMatcher {
    /// argv prefix.
    pub argv_prefix: Vec<String>,
}

/// Expected outcome of a command probe.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    /// Expected exit code.
    pub exit: Option<i32>,
    /// stdout must match.
    pub stdout_regex: Option<String>,
    /// stderr must not match.
    pub stderr_absent_regex: Option<String>,
}

/// Result of evaluating a probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProbeResult {
    /// Hash of the canonical probe.
    pub probe_hash: B3,
    /// Outcome.
    pub outcome: Outcome,
    /// Who evaluated it.
    pub by: ResultBy,
    /// Tree it was evaluated against.
    pub tree: GitOid,
    /// When.
    pub evaluated_at: Ts,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Evidence blob (command output).
    pub evidence: Option<BlobRef>,
    /// Human-readable detail.
    pub detail: String,
}

unit_enum! {
    /// Probe outcome.
    Outcome {
        /// Passed.
        Pass,
        /// Failed.
        Fail,
        /// Could not run (spawn failure, parse error).
        Error,
        /// Timed out.
        Timeout,
        /// Needs approval before it can run.
        NeedsApproval,
        /// Not run (policy or options).
        Skipped,
    }
}

unit_enum! {
    /// Who produced a probe result.
    ResultBy {
        /// Witnessed during the session.
        Witness,
        /// Evaluated while sealing.
        Seal,
        /// Evaluated by the receiver.
        Replay,
    }
}

// ---------------------------------------------------------------- tasks, decisions, attempts

/// A task in the task DAG.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Task {
    /// Task id.
    pub id: Id<Tsk>,
    /// Title (untrusted when agent-authored).
    pub title: String,
    /// Detail.
    pub detail: Option<String>,
    /// Status.
    pub status: TaskStatus,
    /// Dependencies.
    pub deps: Vec<Id<Tsk>>,
    /// Leg that created it.
    pub created_leg: Id<Leg>,
    /// Leg that last updated it.
    pub updated_leg: Id<Leg>,
    /// Claims offered as evidence of completion.
    pub evidence: Vec<Id<Clm>>,
    /// Status history.
    pub history: Vec<TaskEvent>,
}

unit_enum! {
    /// Task status.
    TaskStatus {
        /// Not started.
        Todo,
        /// In progress.
        Doing,
        /// Blocked.
        Blocked,
        /// Done.
        Done,
        /// Dropped.
        Dropped,
    }
}

/// One change in a task's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskEvent {
    /// When.
    pub at: Ts,
    /// Which leg.
    pub leg: Id<Leg>,
    /// New status.
    pub status: TaskStatus,
    /// Optional note.
    pub note: Option<String>,
}

/// A decision with its reasons and constraint strength.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    /// Decision id.
    pub id: Id<Dec>,
    /// What was decided.
    pub text: String,
    /// Why.
    pub why: String,
    /// Rejected alternatives.
    pub alternatives: Vec<Alternative>,
    /// Constraint strength.
    pub constraint: Option<ConstraintKind>,
    /// What it applies to.
    pub scope: Option<Scope>,
    /// Leg that made it.
    pub leg: Id<Leg>,
    /// Who made it.
    pub origin: ClaimOrigin,
    /// Decision this one replaces.
    pub supersedes: Option<Id<Dec>>,
    /// Whether it is still in force.
    pub active: bool,
}

/// A rejected alternative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Alternative {
    /// The alternative.
    pub text: String,
    /// Why it was rejected.
    pub why_rejected: Option<String>,
}

unit_enum! {
    /// Constraint strength. `must` and `must_not` are never dropped from briefs.
    ConstraintKind {
        /// Required.
        Must,
        /// Forbidden.
        MustNot,
        /// Preferred.
        Prefer,
    }
}

/// Something that was tried and did not work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Attempt {
    /// Attempt id.
    pub id: Id<Att>,
    /// What was tried.
    pub what: String,
    /// Outcome.
    pub outcome: AttemptOutcome,
    /// Why it failed or was abandoned.
    pub why: String,
    /// Supporting ledger entries.
    pub evidence: Vec<LedgerRef>,
    /// Leg.
    pub leg: Id<Leg>,
}

unit_enum! {
    /// Outcome of an attempt.
    AttemptOutcome {
        /// Failed.
        Failed,
        /// Abandoned.
        Abandoned,
        /// Partially worked.
        Partial,
    }
}

// ---------------------------------------------------------------- packet

/// Ancestor summary in a packet's lineage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LineageNode {
    /// Packet id.
    pub packet_id: Id<Pkt>,
    /// Manifest hash.
    pub manifest_hash: B3,
    /// Agent that sealed it.
    pub agent: AgentId,
    /// Seal time.
    pub sealed_at: Ts,
    /// Claim status counts at seal time.
    pub status_counts: StatusCounts,
}

/// Claim counts by status.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusCounts {
    /// Verified.
    pub verified: u32,
    /// Holds.
    pub holds: u32,
    /// Stale.
    pub stale: u32,
    /// Refuted.
    pub refuted: u32,
    /// Unverified.
    pub unverified: u32,
    /// Unknown.
    pub unknown: u32,
}

impl StatusCounts {
    /// Increment the counter for a status.
    pub fn add(&mut self, s: ClaimStatus) {
        match s {
            ClaimStatus::Verified => self.verified += 1,
            ClaimStatus::Holds => self.holds += 1,
            ClaimStatus::Stale => self.stale += 1,
            ClaimStatus::Refuted => self.refuted += 1,
            ClaimStatus::Unverified => self.unverified += 1,
            ClaimStatus::Unknown => self.unknown += 1,
        }
    }
    /// Total claims.
    pub fn total(&self) -> u32 {
        self.verified + self.holds + self.stale + self.refuted + self.unverified + self.unknown
    }
}

/// A transcript-extracted or imported statement. Always untrusted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HearsayItem {
    /// Where it came from.
    pub source: OriginSource,
    /// Agent, if known.
    pub agent: Option<AgentId>,
    /// Quote, redacted, at most 240 characters.
    pub quote: String,
    /// Claim extracted from this quote, if any.
    pub claim: Option<Id<Clm>>,
    /// Timestamp in the source, if any.
    pub ts: Option<Ts>,
}

/// Per-scope hashes at seal time, used to compute drift later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DriftBaseline {
    /// Snapshot tree at seal.
    pub anchor_tree: GitOid,
    /// Files referenced by any claim scope.
    pub files: Vec<ScopedFile>,
    /// Symbols referenced by any claim scope.
    pub symbols: Vec<ScopedSymbol>,
}

/// Redaction counts per rule. Never contains secret values.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RedactionReport {
    /// Rules version.
    pub rules_version: String,
    /// Total replacements.
    pub findings: u32,
    /// Replacements per rule id.
    pub per_rule: BTreeMap<String, u32>,
    /// Files excluded from the snapshot as secrets.
    pub excluded_files: u32,
    /// Whether a seal-time overlay map was needed.
    pub overlay: bool,
}

/// Who sealed a packet and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SealInfo {
    /// Sealing agent.
    pub sealed_by: AgentRef,
    /// Seal time.
    pub sealed_at: Ts,
    /// Free-text reason.
    pub reason: Option<String>,
    /// Intended receiver.
    pub target_agent: Option<AgentId>,
    /// Firsthand version.
    pub firsthand_version: String,
}

/// `packet.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PacketBody {
    /// Anchor.
    pub anchor: RepoAnchor,
    /// Legs in this packet, oldest first.
    pub legs: Vec<Leg>,
    /// Ancestor summary.
    pub lineage: Vec<LineageNode>,
    /// Claims.
    pub claims: Vec<Claim>,
    /// Tasks.
    pub tasks: Vec<Task>,
    /// Decisions.
    pub decisions: Vec<Decision>,
    /// Attempts.
    pub attempts: Vec<Attempt>,
    /// Untrusted statements.
    pub hearsay: Vec<HearsayItem>,
    /// Drift baseline.
    pub drift_baseline: DriftBaseline,
    /// Redaction report.
    pub redaction: RedactionReport,
    /// Seal info.
    pub seal: SealInfo,
    /// Extensions; keys must start with `x-`.
    pub extensions: BTreeMap<String, serde_json::Value>,
}

/// `manifest.json`, the signed document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Always `firsthand.packet/1`.
    pub format: String,
    /// Packet id.
    pub packet_id: Id<Pkt>,
    /// Creation time.
    pub created_at: Ts,
    /// Tool identity.
    pub created_by: CreatedBy,
    /// Repository identity.
    pub repo: ManifestRepo,
    /// Anchor commits.
    pub anchor: ManifestAnchor,
    /// Parent packet.
    pub parent: Option<ManifestParent>,
    /// Every part with size and hash.
    pub parts: Vec<PartEntry>,
    /// Redaction summary.
    pub redaction: ManifestRedaction,
    /// Limits.
    pub limits: ManifestLimits,
}

/// Tool that created a packet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreatedBy {
    /// Always `firsthand`.
    pub tool: String,
    /// Version.
    pub version: String,
}

/// Repository identity in a manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManifestRepo {
    /// Repo id.
    pub repo_id: B3,
    /// Object format.
    pub object_format: ObjectFormat,
}

/// Anchor commits in a manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManifestAnchor {
    /// HEAD.
    pub head: Option<GitOid>,
    /// Snapshot commit.
    pub snapshot_commit: GitOid,
    /// Snapshot tree.
    pub snapshot_tree: GitOid,
}

/// Parent reference in a manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManifestParent {
    /// Parent packet id.
    pub packet_id: Id<Pkt>,
    /// Parent manifest hash.
    pub manifest_hash: B3,
}

/// One part of a packet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PartEntry {
    /// Path inside the ZIP.
    pub path: String,
    /// Uncompressed size.
    pub bytes: u64,
    /// BLAKE3 of the uncompressed bytes.
    pub hash: B3,
}

/// Redaction summary in a manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManifestRedaction {
    /// Rules version.
    pub rules_version: String,
    /// Total findings.
    pub findings: u32,
    /// Whether `blobs.json` overlay is present.
    pub overlay: bool,
}

/// Limits declared by the packet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManifestLimits {
    /// Maximum packet size in bytes.
    pub max_bytes: u64,
}

/// `manifest.sig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManifestSig {
    /// Always `ed25519`.
    pub alg: String,
    /// `fp:` plus the first 16 hex of blake3(pubkey).
    pub key_fp: String,
    /// Public key, standard base64.
    pub pubkey: String,
    /// Signature, standard base64.
    pub sig: String,
}

/// Trust classification of a packet's signer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PacketTrust {
    /// Signed by the user's own key.
    Own,
    /// Signed by a key in `trusted_keys.toml`.
    Trusted,
    /// Signed by an unknown key.
    Unknown,
    /// No signature.
    Unsigned,
}

impl PacketTrust {
    /// Whether allowlisted commands may auto-run.
    pub fn allows_auto_run(self) -> bool {
        matches!(self, PacketTrust::Own | PacketTrust::Trusted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_json_shape() {
        let p = Probe::LedgerVerification {
            kind: VerificationKind::Test,
            matcher: None,
            require: LedgerRequire::TestsAllPass,
        };
        let j = serde_json::to_string(&p).unwrap();
        assert_eq!(
            j,
            r#"{"type":"ledger_verification","kind":"test","matcher":null,"require":"tests_all_pass"}"#
        );
        assert_eq!(serde_json::from_str::<Probe>(&j).unwrap(), p);
        assert!(
            serde_json::from_str::<Probe>(r#"{"type":"file_exists","path":"a","x":1}"#).is_err()
        );
    }

    #[test]
    fn agent_ids() {
        assert_eq!(
            serde_json::to_string(&AgentId::GeminiCli).unwrap(),
            "\"gemini-cli\""
        );
        for a in AgentId::ALL {
            assert_eq!(AgentId::parse(a.as_str()), Some(a));
        }
        assert_eq!(
            serde_json::to_string(&CommandRunner::FhRun).unwrap(),
            "\"fh-run\""
        );
    }
}

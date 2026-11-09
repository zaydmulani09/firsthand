# Ledger and witness

Status: normative, format version 1. Part of the Firsthand specification (`spec/`).
Where this document and the code disagree, that is a bug in one of them; please open an issue.

### 5.3 Legs, ledger, witness

```rust
pub struct Leg {
    pub id: Id<Leg>, pub agent: AgentRef, pub agent_session: Option<String>,
    pub started_at: Ts, pub ended_at: Option<Ts>,
    pub anchor_start: Option<SnapshotRef>, pub anchor_end: Option<SnapshotRef>,
    pub parent_packet: Option<PacketRef>, pub outcome: LegOutcome,
    pub ledger: LedgerRange,
}
pub struct AgentRef { pub id: AgentId, pub version: Option<String>, pub model: Option<String> }
pub enum AgentId { ClaudeCode, Codex, GeminiCli, Opencode, Generic }   // JSON kebab-case
pub enum LegOutcome { Open, HandedOff, Completed, Abandoned, Compacted }
pub struct LedgerRange { pub first_seq: u64, pub last_seq: u64, pub head_hash: B3 }
pub struct PacketRef { pub id: Id<Pkt>, pub manifest_hash: B3 }

pub struct LedgerEntry {
    pub v: u8,                       // 1
    pub seq: u64,                    // starts at 0 per leg
    pub prev: B3,                    // hash of previous entry; for seq 0 = blake3("firsthand.ledger/1" + leg_id)
    pub ts: Ts,
    pub leg: Id<Leg>,
    pub kind: EntryKind,
    pub witness: Witness,
    pub body: serde_json::Value,     // kind-specific, integers only, validated per kind
    pub hash: B3,                    // blake3(canonical(entry with hash omitted))
}
pub struct Witness { pub channel: Channel, pub agent: AgentId, pub agent_session: Option<String>, pub tool_use_id: Option<String> }
pub enum EntryKind { LegStart, Command, FileEdit, TestResult, Snapshot, Note, Claim, Decision, Task, Attempt,
                     Ingest, BriefDelivered, ToolOther, LegEnd }
```

Entry body shapes (all fields integers/strings/bools/arrays/objects, no floats):

- `command`: `{ argv: [string] | null, command_text: string, cwd_rel: string, exit_code: int | null, started_at: Ts, duration_ms: int, stdout: BlobRef, stderr: BlobRef, tree_before: GitOid | null, tree_after: GitOid | null, dirty_fp: B3 | null, runner: "bash-tool" | "fh-run", verification_kind: "test"|"build"|"lint"|"typecheck"|"other"|null, test_summary: { runner: string, passed: int, failed: int, skipped: int } | null }`
- `file_edit`: `{ path: string, op: "write"|"edit"|"delete"|"rename", before_blob: GitOid | null, after_blob: GitOid | null, tool: string }`
- `snapshot`: `{ snapshot: SnapshotRef }`
- `note`: `{ text: string }`
- `claim` / `decision` / `task` / `attempt`: `{ op: "create"|"update"|"verify"|"supersede"|"done"|"drop"..., ... full object or patch }`
- `ingest`: `{ agent: AgentId, session_id: string, source_hash: B3, messages: int, tool_events: int, claims_extracted: int }`
- `brief_delivered`: `{ packet_id, agent: AgentId, session_id: string | null }`
- `tool_other`: `{ tool_name: string, summary: string }` (non-Bash, non-edit tool events, truncated)

`BlobRef = { hash: B3, bytes: u64, stored_bytes: u64, truncated: bool, redacted: bool }`. Blob store truncation policy: keep first 32 KiB and last 32 KiB with an explicit elision marker line; `bytes` is the original size, `hash` is the hash of the **stored** (already redacted, possibly truncated) content.

Witness integrity rule: entries created via `hook` channel must come from `firsthand hook <agent> <event>` parsing harness JSON. Entries via `cli`, `mcp`, `ingest`, `import` can never be given channel `hook` or `run`. The channel is assigned by the entry point code, never read from input.


### 9.1 Chain and append

- Each entry's `hash = blake3(canonical(entry without hash))`; `prev` links; seq starts at 0; seq 0's `prev = blake3("firsthand.ledger/1" || leg_id)`.
- `fh_ledger::verify_chain(path)` returns first bad seq and reason. Used by `inspect`, `receive`, `doctor`, `reindex`.
- Blob store: write temp, hash, rename into `blobs/<aa>/<hash>`. Idempotent. Redact before hashing (so the stored hash describes stored bytes). Truncation policy per 5.3.

### 9.2 Hook fast path

`firsthand hook <agent> <event>` reads harness JSON from stdin (or the last argv for Codex-style `notify` hooks), classifies, appends, exits. Budget in 3.4. Rules:
- No tokio, no SQLite, no network, no tree hashing, except for verification-pattern commands (Pre: stamp `tree_before` via `tree_only`; Post: stamp `tree_after` and parse test output). Pairing through `pending/<tool_use_id>.json`, pruned after 1 hour.
- A hook failure must never break the agent: exit 0 on internal errors (log to `<store>/hook-errors.log`, capped), except when the harness protocol requires a block decision (Firsthand never blocks tool calls).
- Stop and PreCompact events: take an `Auto`/`PreCompact` snapshot when `auto_snapshot_on_stop` and the tree changed since the last auto snapshot (compare cheap fingerprint first).
- SessionStart: if `pending.json` names this agent (or `any`), print the bootstrap prompt to stdout (for harnesses that add SessionStart stdout to context), record `brief_delivered`, clear the pending marker. Always begins a new leg and links `parent_packet` from the pending marker.
- SessionEnd or last Stop: write `leg_end`.

### 9.3 Verification-command detection and test parsers

Built-in verification patterns (argv prefix match, case-insensitive on Windows): `cargo test|build|check|clippy|nextest`, `npm|pnpm|yarn (run )?test|build|lint|typecheck`, `npx vitest|jest|tsc|eslint`, `pytest`, `python|py -m pytest|unittest`, `go test|build|vet`, `make test|build|check`, `dotnet test|build`, `gradle|./gradlew test|build`, `mvn test|package`. Parsers (each with fixture tests from real captured output formats, synthetic content): cargo test (`test result: ok. N passed; M failed; K ignored`), pytest (`N passed, M failed, K skipped in Xs`), vitest/jest summary lines, `go test` (`ok`/`FAIL`/`--- FAIL`), generic exit-code fallback (counts null). Unknown format yields `test_summary: null` and exit-code semantics only.

---

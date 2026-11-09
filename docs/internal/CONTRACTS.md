# Contracts (frozen at G0)

This file is the interface agreement between parallel tracks. The product spec is
`docs/internal/SPEC.md` (sections are referenced as "spec 8.3" and so on). Rules for
agents are in `AGENT_RULES.md`. Your track file is `docs/internal/tracks/<X>.md`.

If something here is impossible or wrong, do not work around it in another track's
files. Append a request to `CONTRACT_AMENDMENTS.md` (what, why, proposed signature)
and continue with the smallest local alternative. The orchestrator resolves amendments.

## 0. Environment (Windows dev machine, also valid on Linux and macOS)

- Rust must use MSVC on Windows. In every shell that builds, set
  `$env:RUSTUP_TOOLCHAIN = "stable-x86_64-pc-windows-msvc"` (PowerShell) or
  `export RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-msvc` (bash). The main checkout uses a
  rustup directory override; worktrees do not inherit it.
- Each track uses its own target dir and at most 2 build jobs (the machine has 12 GB RAM and
  five tracks build at once): `CARGO_TARGET_DIR=C:\Users\<you>\fhwt\target-<letter>`,
  `CARGO_BUILD_JOBS=2`. Your track prompt gives the exact path.
- Never put a target dir inside OneDrive. Worktrees live under `%USERPROFILE%\fhwt\<track>`.
- Python is `py`. Node 24 and npm 11 are installed. No paid services, no network from the
  product, no model prompts (R1 to R3).
- `cargo xtask check --phase0` runs fmt, clippy, tests and both lints. Use it before reporting.

## 1. Crate map and owners

| Crate | Owner | Depends on (allowed) |
|---|---|---|
| fh-core | orchestrator (frozen) | none of ours |
| fh-git | A (Phase 0 essentials by orchestrator) | fh-core |
| fh-store | A | fh-core |
| fh-ledger | A | fh-core, fh-git |
| fh-sem | B | fh-core, fh-git |
| fh-claims | B | fh-core, fh-git, fh-sem (NOT fh-ledger: probes take `&[LedgerEntry]`) |
| fh-redact | C | fh-core |
| fh-packet | C | fh-core, fh-redact |
| fh-brief | C | fh-core |
| fh-adapters | D | fh-core |
| fh-fakeagent | D | fh-core, fh-adapters |
| fh-ui | E | fh-core (and fh-flows after integration) |
| fh-flows | F (new crate, see DEVIATIONS.md D1) | everything except cli, mcp, ui, bench |
| fh-mcp | F | fh-core, fh-flows |
| fh-cli | F | everything |
| fh-bench | G | fh-core, fh-git, fh-testkit; fh-flows for strategy S4 only |
| fh-testkit | orchestrator, any track may add helpers through an amendment | fh-core |
| xtask | orchestrator; C owns `vectors` content, E owns `build-ui` | |

Cross-track data types live in `fh-core` (`model.rs`, `iface.rs`) and are frozen. You may add
private types freely inside your own crates.

## 2. fh-core (exists, frozen)

Modules: `canon` (JCS, integers only: `canonical_bytes`, `hash_canonical`, `check_integers`),
`hash` (`B3`, `GitOid`), `id` (`Id<T>`, markers `Pkt Clm Tsk Dec Att Snp`, `Leg` struct is
also a marker; `IdGen::{from_env, seeded, new_id, new_id_salted}`), `time` (`Ts`, `Clock`,
`SystemClock` honoring `FIRSTHAND_FAKE_TIME`, `FixedClock`), `model` (all spec 5 and 6.3
types), `iface` (`HookEvent`, `ToolCall`, `HookOutcome`, `IngestedSession`, `DriftReport`,
`HeadRelation`, `PathChange`, `SymbolChange`, `Cause`, `ClaimAffected`, `ClaimEval`,
`Redacted`, trait `Redactor`), `policy` (`Policy`, `ExecDecision`, `program_key`, `SAFE_ENV`,
`RepoConfig`), `paths` (`normalize_rel`, `is_reserved_windows_name`, `case_collisions`,
`scrub_identity`), `store` (`StoreLayout`, `UserLayout`, `user_config_dir`, `briefs_dir`),
`error` (`Error`, `ExitCode`), and trait `SecretScanner` in the crate root.

JSON shape notes that differ from a naive reading of the spec:
- `Scope`, `Probe`, `HeadRelation`, `Cause` are internally tagged with `"type"` (snake_case).
- Ids are `prefix + 32 lowercase hex` (UUIDv7 simple form). Short form: prefix + last 8.
- In seeded mode, ids created in different processes must pass a salt (`new_id_salted`).
- Ledger bodies are typed structs in `model.rs` (`CommandBody`, `FileEditBody`, `NoteBody`,
  `IngestBody`, `BriefDeliveredBody`, `ToolOtherBody`, `SnapshotBody`, `OpBody`), stored as
  `serde_json::Value` in `LedgerEntry.body`.

## 3. fh-git (A, Phase 0 parts exist)

Exists: `GitRunner` (hardened; `run`, `run_with`, `run_str`, `run_opt`, `output_with`,
`version`), `Repo::{discover, store, git_version, head, branch, upstream, repo_id,
worktree_info, index_path, read_blob, ls_tree, blob_at, has_object, dirty_summary,
snapshot_candidates, tree_only, snapshot_dirty, list_snapshot_refs, head_relation,
diff_name_status}`, `parse_ls_tree`, `parse_status_v2`, `parse_name_status`, `split_nul`.

Track A implements (signatures frozen in `advanced.rs`): `bundle_create`,
`bundle_materialize`, `worktree_add_detached`, `worktree_remove`, `restore_to`,
`restore_apply(snapshot, RestoreMode) -> SnapshotRef` (returns the PreRestore snapshot),
`rotate_auto(keep) -> u32`. A may add: a batched blob reader (`cat-file --batch`) as
`Repo::blob_reader() -> BlobReader` with `read(&GitOid) -> Result<Vec<u8>>`, and anything
else needed, as additive public API.

## 4. fh-ledger (A)

```rust
pub fn genesis_prev(leg: &Id<Leg>) -> B3;                 // blake3("firsthand.ledger/1" || leg_id)
pub fn entry_hash(e: &LedgerEntry) -> Result<B3>;          // blake3(canonical(entry without hash))
pub fn read_ledger(path: &Path) -> Result<Vec<LedgerEntry>>;   // tolerant of a torn last line (reported)
pub struct ChainReport { pub entries: u64, pub ok: bool, pub first_bad: Option<(u64, String)> }
pub fn verify_chain(entries: &[LedgerEntry]) -> ChainReport;
pub struct Appender { .. }                                   // locked appends, fsync, torn-line repair
impl Appender {
    pub fn open(store: &StoreLayout) -> Result<Appender>;
    pub fn append(&self, leg: &Id<Leg>, kind: EntryKind, witness: Witness,
                  body: serde_json::Value, clock: &dyn Clock) -> Result<LedgerEntry>;
    pub fn start_leg(&self, agent: AgentRef, session: Option<String>, parent: Option<PacketRef>,
                     channel: Channel, clock: &dyn Clock, ids: &IdGen) -> Result<Id<Leg>>;
    pub fn ensure_leg(&self, agent: AgentRef, session: Option<String>, channel: Channel,
                      clock: &dyn Clock, ids: &IdGen) -> Result<Id<Leg>>; // reuse open leg of same agent+session
    pub fn end_leg(&self, leg: &Id<Leg>, outcome: LegOutcome, clock: &dyn Clock) -> Result<()>;
    pub fn current_leg(&self) -> Result<Option<Id<Leg>>>;
    pub fn list_legs(&self) -> Result<Vec<Id<Leg>>>;
    pub fn leg(&self, leg: &Id<Leg>) -> Result<Leg>;           // projected from its entries
}
pub struct BlobStore { .. }
impl BlobStore {
    pub fn open(store: &StoreLayout) -> Result<BlobStore>;
    pub fn put(&self, raw: &[u8], redactor: &dyn Redactor) -> Result<BlobRef>; // redact, truncate 32K+32K, hash stored bytes
    pub fn get(&self, hash: &B3) -> Result<Vec<u8>>;
}
pub fn detect_verification(argv: &[String], policy: &Policy) -> Option<VerificationKind>;
pub fn split_command(text: &str) -> Vec<String>;       // best-effort argv from shell text, no shell semantics
pub fn parse_test_output(argv: &[String], stdout: &str, stderr: &str) -> Option<TestSummary>;
pub struct HookCtx<'a> { pub repo: &'a Repo, pub policy: &'a Policy, pub clock: &'a dyn Clock,
    pub ids: &'a IdGen, pub redactor: &'a dyn Redactor, pub scanner: &'a dyn SecretScanner }
pub fn handle_event(ev: &HookEvent, ctx: &HookCtx) -> Result<HookOutcome>;  // spec 9.2, incl. SessionStart arming
pub struct Pending { pub packet_id: Id<Pkt>, pub manifest_hash: B3, pub agent: Option<AgentId>,
    pub bootstrap: String, pub brief_path: String, pub armed_at: Ts }   // serde, pending.json
pub fn arm(store: &StoreLayout, p: &Pending) -> Result<()>;
pub fn take_pending(store: &StoreLayout, agent: AgentId) -> Result<Option<Pending>>; // clears on match
pub fn disarm(store: &StoreLayout) -> Result<bool>;
pub struct CommandRecord { pub argv: Vec<String>, pub command_text: String, pub cwd_rel: String,
    pub exit_code: Option<i32>, pub started_at: Ts, pub duration_ms: u64, pub stdout: Vec<u8>,
    pub stderr: Vec<u8>, pub tree_before: Option<GitOid>, pub tree_after: Option<GitOid> }
pub fn record_command(ctx: &HookCtx, leg: &Id<Leg>, rec: CommandRecord, channel: Channel,
    runner: CommandRunner) -> Result<LedgerEntry>;      // used by hooks and `firsthand run`
```
Hook rules (spec 9.2): no tokio, no SQLite, exit 0 on internal errors with a capped
`hook-errors.log`. The fast path never computes a tree except for verification commands and
Stop/PreCompact auto snapshots. Channel is set by the caller code path, never read from input.

## 5. fh-store (A)

```rust
pub struct Index { .. }                                   // SQLite, WAL, FK on, DDL = spec 7
impl Index {
    pub fn open(path: &Path) -> Result<Index>;             // creates and migrates
    pub fn user_version(&self) -> Result<u32>;
    pub fn ingest_entries(&mut self, repo_id: &B3, entries: &[LedgerEntry]) -> Result<()>;
    pub fn record_packet(&mut self, row: &PacketRow) -> Result<()>;
    pub fn packets(&self) -> Result<Vec<PacketRow>>;
    pub fn search(&self, query: &str, kind: Option<EntryKind>, limit: u32) -> Result<Vec<SearchHit>>;
}
pub fn reindex(store: &StoreLayout, repo_id: &B3) -> Result<ReindexReport>;   // drop, rebuild, verify chains
pub fn gc(store: &StoreLayout, repo: &Repo, dry_run: bool, keep_auto: u32) -> Result<GcReport>;
```
`PacketRow`, `SearchHit`, `ReindexReport`, `GcReport` are plain serde structs defined by A.

## 6. fh-sem (B)

```rust
pub enum Lang { Rust, TypeScript, Tsx, JavaScript, Python, Go }
pub fn language_for(path: &str) -> Option<Lang>;
pub struct Symbol { pub name: String, pub qualified_name: String, pub kind: SymbolKind,
    pub path: String, pub start_line: u32, pub end_line: u32, pub body_hash: B3, pub sig_hash: B3 }
pub fn index_source(path: &str, source: &[u8]) -> Result<Vec<Symbol>, SemError>; // SemError::{Unsupported, Parse}
pub struct SymbolIndex { .. }  // by path, by (name, kind)
pub fn index_tree(repo: &Repo, tree: &GitOid, paths: Option<&[String]>) -> Result<SymbolIndex>;
pub fn semantic_diff(repo: &Repo, a: &GitOid, b: &GitOid) -> Result<SemanticDiff>;
```

## 7. fh-claims (B)

```rust
pub fn probe_hash(p: &Probe) -> B3;
pub enum Sandbox { AtCurrent, Worktree }
pub enum Approval { Once, No, Always }
pub trait Approver { fn approve(&self, argv: &[String], cwd_rel: &str, env_keys: &[String]) -> Approval; }
pub struct EvalOptions { pub rerun: bool, pub approve: BTreeSet<String> /* claim ids */,
    pub sandbox: Sandbox }
pub struct EvalCtx<'a> { pub repo: &'a Repo, pub anchor: &'a RepoAnchor, pub baseline: &'a DriftBaseline,
    pub receiver_tree: GitOid, pub ledger: &'a [LedgerEntry], pub policy: &'a Policy,
    pub trust: PacketTrust, pub opts: EvalOptions, pub approver: Option<&'a dyn Approver>,
    pub clock: &'a dyn Clock, pub redactor: &'a dyn Redactor }
pub fn evaluate(claim: &Claim, ctx: &EvalCtx) -> ClaimEval;            // spec 8.3, never panics
pub fn evaluate_all(claims: &[Claim], ctx: &EvalCtx) -> Vec<ClaimEval>;
pub fn drift(ctx: &EvalCtx, claims: &[Claim]) -> Result<DriftReport>;  // spec 8.4
pub fn scope_for_files(repo: &Repo, tree: &GitOid, files: &[String], symbols: &[String]) -> Result<Scope>;
pub fn baseline_for(repo: &Repo, tree: &GitOid, claims: &[Claim]) -> Result<DriftBaseline>;
pub fn derive_witnessed_claims(ledger: &[LedgerEntry], clock: &dyn Clock, ids: &IdGen) -> Vec<Claim>; // spec 2.3 (1)
pub fn extract_claims(session: &IngestedSession, leg: &Id<Leg>, ledger_ref: &LedgerRef,
    repo: &Repo, tree: &GitOid, clock: &dyn Clock, ids: &IdGen) -> (Vec<Claim>, Vec<HearsayItem>); // spec 8.6
pub mod exec {
    pub struct ExecSpec { pub argv: Vec<String>, pub cwd: PathBuf, pub shell: bool, pub timeout: Duration,
        pub max_output: u64, pub env: Vec<(String, String)> }
    pub struct ExecResult { pub exit_code: Option<i32>, pub stdout: Vec<u8>, pub stderr: Vec<u8>,
        pub duration_ms: u64, pub timed_out: bool, pub truncated: bool }
    pub fn scrubbed_env(policy: &Policy) -> Vec<(String, String)>;
    pub fn run(spec: &ExecSpec) -> Result<ExecResult>;   // process-tree kill on timeout
}
```
Claims auto-derived from witnessed verification commands are produced at seal time by
`derive_witnessed_claims` (a pure projection of the ledger), not written by hooks.
Seal-time evaluation uses `ResultBy::Seal`; receive uses `ResultBy::Replay`.

## 8. fh-redact, fh-packet, fh-brief (C)

```rust
// fh-redact
pub const RULES_VERSION: &str = "1";
pub struct StandardRedactor { .. }      // implements Redactor and SecretScanner
impl StandardRedactor { pub fn new(home: Option<String>, user: Option<String>) -> Self;
                        pub fn for_host() -> Self; }
pub fn rule_ids() -> &'static [&'static str];

// fh-packet
pub struct PacketInput { pub packet_id: Id<Pkt>, pub created_at: Ts, pub body: PacketBody,
    pub ledgers: Vec<(Id<Leg>, Vec<LedgerEntry>)>, pub blobs: Vec<(B3, Vec<u8>)>,
    pub bundle: Option<Vec<u8>>, pub brief_md: Option<String>, pub parent: Option<ManifestParent>,
    pub max_bytes: u64 }
pub fn write_packet(input: &PacketInput, key: Option<&KeyPair>) -> Result<Vec<u8>>; // deterministic
pub struct OpenedPacket { pub manifest: Manifest, pub manifest_hash: B3, pub sig: Option<ManifestSig>,
    pub trust: PacketTrust, pub key_fp: Option<String>, pub body: PacketBody,
    pub ledgers: Vec<(Id<Leg>, Vec<LedgerEntry>)>, pub blobs: BTreeMap<String, Vec<u8>>,
    pub bundle: Option<Vec<u8>>, pub brief_md: Option<String> }
pub fn open_packet(bytes: &[u8], trust: &TrustStore, own: Option<&str /*own fp*/>) -> Result<OpenedPacket>; // spec 6.4 + 6.5, Error::Integrity
pub fn inspect(bytes: &[u8], trust: &TrustStore, own: Option<&str>) -> InspectReport;  // never panics
pub mod keys { pub struct KeyPair; impl KeyPair { generate, load(&UserLayout), save(&UserLayout),
    public_b64, fingerprint, sign(&[u8]) }
    pub fn fingerprint(pubkey: &[u8]) -> String;   // "fp:" + 16 hex of blake3
    pub struct TrustStore; impl TrustStore { load(&UserLayout), add, remove, list, classify } }

// fh-brief
pub struct BriefInput<'a> { pub packet_id: &'a Id<Pkt>, pub body: &'a PacketBody,
    pub evals: &'a [ClaimEval], pub drift: &'a DriftReport, pub trust: PacketTrust,
    pub key_fp: Option<&'a str>, pub chain_ok: bool, pub ledger_entries: u64,
    pub target: AgentId, pub budget_tokens: u32 }
pub struct Brief { pub markdown: String, pub json: serde_json::Value, pub est_tokens: u32, pub over_budget: bool }
pub fn compile(input: &BriefInput) -> Brief;                       // spec 11, deterministic
pub fn bootstrap_prompt(packet_id: &Id<Pkt>, target: AgentId) -> String;   // spec 11.4
pub fn agents_md_insert(existing: &str, packet_id: &Id<Pkt>) -> String;    // spec 11.5
pub fn agents_md_remove(existing: &str) -> String;
```

## 9. fh-adapters (D)

Trait exactly as spec 10.1 plus these concrete types:
```rust
pub struct HostEnv { pub home: Option<PathBuf>, pub path_var: Option<OsString>, pub env: BTreeMap<String, String> }
pub struct Detection { pub installed: bool, pub version: Option<String>, pub config_paths: Vec<PathBuf>, pub notes: Vec<String> }
pub struct RepoCtx { pub root: PathBuf, pub common_dir: PathBuf, pub firsthand_cmd: String }
pub enum HookScope { Repo, User }
pub struct SessionRef { pub agent: AgentId, pub id: String, pub path: PathBuf, pub modified: Option<Ts>, pub cwd: Option<String> }
pub struct IngestOpts { pub max_bytes: u64, pub max_messages: u32 }
pub struct FileEdit { pub path: PathBuf, pub before: Option<Vec<u8>>, pub after: Vec<u8>, pub scope: HookScope, pub description: String }
pub struct HookPlan { pub agent: AgentId, pub edits: Vec<FileEdit>, pub manual_steps: Vec<String> }
pub type ConfigPlan = HookPlan;
pub struct LaunchSpec { pub argv: Vec<String>, pub env: Vec<(String, String)>, pub cwd: PathBuf, pub verified: bool }
pub struct BriefRef { pub packet_id: String, pub brief_path: String, pub bootstrap: String }
pub fn all() -> Vec<Box<dyn AgentAdapter>>;
pub fn get(id: AgentId) -> Box<dyn AgentAdapter>;
pub fn is_firsthand_hook_command(cmd: &str) -> bool;   // matches `firsthand hook` and `firsthand.exe" hook`
pub fn unified_diff(path: &str, before: &[u8], after: &[u8]) -> String;
pub fn apply_plan(plan: &HookPlan) -> Result<Vec<PathBuf> /*backups*/>;  // atomic write + timestamped .bak
pub fn uninstall_plan(repo: &RepoCtx, scope: HookScope) -> Result<Vec<HookPlan>>; // removes only tagged entries
```
Injected hook commands: `<firsthand_cmd> hook <agent-id> <EventName>`. MCP entries: `<firsthand_cmd> mcp serve`.
Hook stdin parsing lives in adapters (`parse_hook_event`); the ledger never parses harness JSON.

## 10. fh-ui (E)

E defines `pub trait DataSource: Send + Sync` in fh-ui with one method per API route in spec
14.2 returning serde types E defines (built from fh-core types), a fixture-backed implementation
used only in fh-ui tests, `pub async fn serve(opts: UiOptions, data: Arc<dyn DataSource>) -> Result<()>`,
and `pub fn export_static(dir: &Path, data: &dyn DataSource) -> Result<()>`. The real
`DataSource` is implemented in fh-flows by F at integration.

## 11. fh-flows, fh-mcp, fh-cli (F)

fh-flows holds the orchestration used by the CLI, MCP server, dashboard and benchmark:
`seal`, `receive`, `verify`, `replay`, `status`, `drift`, `diff`, `lineage`, `restore`,
`import_md`, plus the real `DataSource`. fh-mcp is protocol plus tool dispatch over fh-flows.

## 12. Exit codes and errors

`fh_core::ExitCode` is the only source of exit codes (spec 12). Errors: lowercase, no trailing
period, say what to do. Never swallow errors except in the hook fast path (logged, exit 0).

## 13. Copy and placeholders

`cargo xtask lint-copy`: no U+2014 or U+2013 anywhere in shipped text, no banned phrases (the
noun "harness" is allowed; the verb is not). `cargo xtask lint-placeholders`: no placeholder
markers in shipped paths. `--phase0` tolerates `NotYetImplemented` until Track A merges.

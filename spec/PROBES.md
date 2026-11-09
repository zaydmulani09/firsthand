# Claims, probes and evaluation

Status: normative, format version 1. Part of the Firsthand specification (`spec/`).
Where this document and the code disagree, that is a bug in one of them; please open an issue.

### 2.3 Claim categories and auto-derivation

Claims come from three sources:

1. **Auto-derived from witnessed events (the main path, zero agent effort).** After a witnessed command that matches a verification pattern (test, build, lint, typecheck, per `policy.toml` and built-in defaults for cargo, npm/pnpm/yarn, pytest, go, make, dotnet, gradle, mvn), create a claim `command_succeeded` or `command_failed` with a `LedgerVerification` plus `Command` probe, scope `repo`, tree oid recorded, tier WITNESSED. Test output is parsed into counts (`passed`, `failed`, `skipped`).
2. **Agent-declared** via `firsthand claim add`, the MCP tool `firsthand_claim`, or `firsthand run --claim "..."`. Starts HEARSAY unless a passing probe is attached.
3. **Extracted from transcript narration (ingest).** A deterministic extractor scans the final assistant messages of the session for assertions. Categories: `tests_pass`, `build_ok`, `lint_ok`, `file_created(path)`, `file_modified(path)`, `file_deleted(path)`, `symbol_added(name)`, `refactor_done`, `bug_fixed`, `task_done`, `other`. Low confidence, always HEARSAY initially. Auto-attach safe probes where a correlate exists:
   - `file_created(path)` gets `FileExists`; `file_deleted(path)` gets `FileAbsent`.
   - `tests_pass`, `build_ok`, `lint_ok` get a `LedgerVerification` probe that looks up the latest witnessed verification command of that kind. No such command in the ledger yields `unverified: no test run witnessed`. A failing last run yields `refuted` with the ledger event cited.
   - `symbol_added(name)` with backticked identifiers gets `SymbolDefined` when the name resolves uniquely in the symbol index.
   - Everything else stays HEARSAY with no probe. Never invent a probe.

### 5.4 Claims, scope, probes, results

```rust
pub struct Claim {
    pub id: Id<Clm>, pub text: String, pub category: ClaimCategory,
    pub origin: ClaimOrigin,            // who asserted it
    pub scope: Scope, pub probe: Option<Probe>, pub results: Vec<ProbeResult>,
    pub created_at: Ts, pub leg: Id<Leg>, pub ledger_ref: LedgerRef,
}
pub enum ClaimCategory { CommandSucceeded, CommandFailed, TestsPass, BuildOk, LintOk, FileCreated, FileModified,
                         FileDeleted, SymbolAdded, RefactorDone, BugFixed, TaskDone, Branch, Other }
pub struct ClaimOrigin { pub source: OriginSource, pub agent: Option<AgentId>, pub confidence_permille: u16 }
pub enum OriginSource { Witnessed, AgentDeclared, Human, TranscriptExtracted, Imported }

pub enum Scope {
    Repo { tree: GitOid },                                  // depends on the whole tree
    Items { files: Vec<ScopedFile>, symbols: Vec<ScopedSymbol> },
}
pub struct ScopedFile   { pub path: String, pub blob: Option<GitOid> }                 // None = file absent at anchor
pub struct ScopedSymbol { pub path: String, pub name: String, pub kind: SymbolKind, pub body_hash: B3, pub sig_hash: B3 }

pub enum Probe {
    FileExists { path: String },
    FileAbsent { path: String },
    FileHash { path: String, blob: GitOid },
    FileContains { path: String, pattern: String, min: u32 },        // regex (Rust regex syntax), min matches
    GlobCount { glob: String, op: CmpOp, n: u32 },
    GitCommitExists { oid: GitOid },
    GitBranchAt { branch: String, oid: GitOid },
    GitPathClean { path: String },
    SymbolDefined { path: Option<String>, name: String, kind: Option<SymbolKind> },
    SymbolAbsent { name: String },
    SymbolHash { path: String, name: String, body_hash: B3 },
    LedgerVerification { kind: VerificationKind, matcher: Option<CommandMatcher>, require: LedgerRequire },
    Command { argv: Vec<String>, cwd_rel: String, shell: bool, timeout_s: u32, expect: Expect },
    All { probes: Vec<Probe> },
}
pub enum LedgerRequire { ExitZero, TestsAllPass }
pub struct CommandMatcher { pub argv_prefix: Vec<String> }
pub struct Expect { pub exit: Option<i32>, pub stdout_regex: Option<String>, pub stderr_absent_regex: Option<String> }

pub struct ProbeResult {
    pub probe_hash: B3, pub outcome: Outcome, pub by: ResultBy, pub tree: GitOid,
    pub evaluated_at: Ts, pub duration_ms: u64, pub evidence: Option<BlobRef>, pub detail: String,
}
pub enum Outcome { Pass, Fail, Error, Timeout, NeedsApproval, Skipped }
pub enum ResultBy { Witness, Seal, Replay }
```

### 8.2 Probe execution

Static probes (everything except `Command`) run in-process against a **tree**, never against the live working directory, using batched reads (`git cat-file --batch`, `git ls-tree -r -z`). They execute nothing and need no approval.

`LedgerVerification { kind, matcher, require }`: find the latest witnessed `command` ledger entry in the packet (or local ledger) whose `verification_kind` equals `kind` and whose argv matches `matcher`. `ExitZero` passes when that entry's `exit_code == 0`. `TestsAllPass` requires `test_summary.failed == 0` and `passed > 0`. Outcome carries the entry id and the tree it ran against. This probe is how a hearsay "tests pass" is cross-checked with zero execution.

`Command` probes execute a program. Execution rules:
- Mode `at_current` (default): run in the repo working directory so dependencies and caches are available. Mode `--sandbox worktree`: run in a temp detached worktree at the anchor snapshot (hermetic, but fails when deps are untracked; documented trade-off).
- Gate order: (1) packet trust must be `own` or `trusted` for any auto-run, else `NeedsApproval` (or `Skipped` for unsigned/unknown in non-interactive mode). (2) argv must match `policy.exec.allow` prefixes and not hit `deny_always`; no match means `NeedsApproval`. (3) `shell: true` requires `allow_shell` and explicit per-command approval even then. (4) Interactive approval shows the exact argv, cwd, and env keys, with choices `[y]es once / [N]o / [a]lways for this argv in this repo` (always writes to local `policy.toml`). Non-interactive: `--approve-commands <claim ids>` only.
- Environment: scrubbed to the safe set (5.6). stdin closed. Output capped. Timeout enforced with process-tree kill. Exit code, duration, and capped output stored as an evidence blob (redacted).
- Evidence outcome rules: `Pass` iff `expect` satisfied. Spawn failure is `Error`. Timeout is `Timeout`.
- MCP-initiated commands obey the same gates. An agent can never self-approve.

### 8.3 Claim evaluation (`fh_claims::evaluate`)

Inputs: claim, packet context (anchor, ledger), receiver tree oid, policy, options (`rerun: bool`, `approve: set`).

```
changed = scope_changes(claim.scope, receiver_tree)       // exact list: files and symbols whose hash differs
contradiction = ledger_contradiction(claim)               // witnessed evidence against the claim on the same scope
if contradiction: return Refuted(evidence=contradiction)
if claim.probe is None: return Unverified("no probe attached; would be settled by: <suggested probe>")
last_pass = latest witnessed/seal Pass result whose tree scope-matches the anchor
if changed.is_empty() and last_pass exists and not options.rerun:
    return Holds(tier=Witnessed)
// scope changed, or rerun requested
match run_probe(claim.probe, receiver_tree, policy, options):
    Pass            -> Verified(tier=Replayed)
    Fail            -> Refuted(evidence)
    NeedsApproval   -> Stale(changed) if changed non-empty else Unknown("needs approval: <argv>")
    Error | Timeout -> Unknown(detail)
    (not runnable under gates) -> Stale(changed) if changed non-empty else Holds
```

`scope_changes` for `Scope::Repo` compares the tree oid (changed means *any* tracked-or-untracked-nonignored file differs). For `Scope::Items`, each `ScopedFile` compares blob oids at the receiver tree; each `ScopedSymbol` re-extracts the symbol from the receiver tree via `fh-sem` and compares `body_hash` and `sig_hash`. A symbol that moved files but has identical hashes is **unchanged** (match by `(name, kind)` repo-wide when the original path no longer contains it). Edits elsewhere in the same file do not change a symbol's `body_hash`. This is the precision win over file-level and commit-distance staleness.

Test and build claims are repo-scoped by nature: any source change makes them stale unless re-run. Offer narrowing with `--scope <glob>` on `firsthand run` and `firsthand claim add`. Say so in the output, honestly.

### 8.4 Drift report (`fh_claims::drift`)

```
DriftReport { head_relation: Same|Ahead(n)|Behind(n)|Diverged{ahead,behind}|Unrelated|AnchorMissing,
              commits_since_anchor: u32, files_changed: Vec<PathChange>, symbols_changed: Vec<SymbolChange>,
              claims_affected: Vec<{claim_id, causes: Vec<Cause>}>, branch_changed: bool, worktree_dirty_now: bool }
```
Use `git rev-list --left-right --count`, `git merge-base`, `git diff --name-status -z`. `AnchorMissing` (the receiver clone lacks the anchor commit) degrades gracefully: static probes still run against the current tree, scope hashes compare against the baseline stored in the packet, and the report says exactly what could not be computed.

### 8.5 Semantic layer (`fh-sem`)

- `index_tree(tree, paths) -> SymbolIndex`: for each supported file (Rust, TypeScript/TSX, JavaScript, Python, Go), parse with tree-sitter and emit `Symbol { name, qualified_name, kind (fn|method|class|struct|enum|trait|interface|type|const|module), path, range, body_hash, sig_hash }`. `body_hash` = BLAKE3 of the node text with comments removed and whitespace runs collapsed (formatting-insensitive). `sig_hash` = hash of name, parameter list, and return type text.
- Only parse files that changed or are referenced by a claim scope. Cache by blob oid in `index.db`. Parallelize with `rayon`.
- `semantic_diff(tree_a, tree_b) -> SemanticDiff { files: added/removed/modified/renamed, symbols: added/removed/body_changed/sig_changed/moved }`. Rename detection by `git diff -M` plus symbol hash matching.
- Unsupported languages: file scope only, reported as such. Parse errors: fall back to file scope for that file and say so in the report.
- Grammar build order if time pressure hits: Rust, TypeScript/JavaScript, Python first; Go last.

### 8.6 Claim extraction from transcripts (`fh_claims::extract`)

Deterministic, no LLM. Input: normalized `IngestedSession` messages. Consider only assistant messages in the final turn group and any message flagged as a summary. Sentence-split. Match category patterns (anchored regex sets, English only in v1, case-insensitive): e.g. `(all|every)\s+(the\s+)?tests?\s+(are\s+)?(now\s+)?(pass|passing|green)`, `(build|compil\w+)\s+(succeed\w*|is\s+clean|passes)`, `(created|added|wrote)\s+`?([\w./\\-]+\.\w+)`?`, `(fixed|resolved)\s+`, `(implemented|added)\s+`(\w+)``. Output `Claim` with `origin = TranscriptExtracted`, `confidence_permille` set by pattern specificity (named file or symbol: 600; generic: 300), then attach probes per 2.3. Cap at 40 extracted claims per leg, prefer those with probes. Store the source quote (max 240 chars, redacted) in `hearsay`.

---

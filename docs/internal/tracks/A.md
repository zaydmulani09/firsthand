<!-- Derived from the Firsthand master build spec. Internal working document. -->

# Track A

**Track A: Store, ledger, git advanced** (branch `track/a-store`, ~7 hours)
Owns: `crates/fh-git` (beyond Phase 0 essentials), `crates/fh-store`, `crates/fh-ledger`.
Deliver: remaining `fh-git` (bundle create and materialize, worktree add/remove, rev-list/merge-base/diff-name-status parsing, restore `--to/--apply/--exact`, auto-snapshot rotation, fingerprint, submodule handling, large/secret exclusion plumbing hooks); `fh-store` (DDL, migrations with tests, projection from ledger and packets, FTS, `reindex`, `gc`); `fh-ledger` (Appender with lock, torn-line recovery, `verify_chain`, blob store with truncation and redaction hook, verification-command detection, test-output parsers with fixtures, hook-pairing store, witness event types, hook fast-path core function `handle_event(HookEvent) -> Result<()>` that Track D's adapters call).
Done when: AC-03, AC-04, AC-05, T20 style concurrent append test (16 processes x 500 appends, no loss, chain valid), snapshot-unchanged-state property tests on weird repos, reindex round trip, parser fixture tests.

## Orchestrator notes (G0)

- Public API to implement: `CONTRACTS.md` sections 3, 4, 5. `fh-git` Phase 0 code already exists; extend it, do not rewrite it.
- SessionStart arming and leg linking are yours (DEVIATIONS D3): `handle_event` reads `pending.json` via `take_pending`, prints the bootstrap once, appends `brief_delivered`, starts a new leg with `parent_packet`.
- Replace every `NotYetImplemented` in `crates/fh-git/src/advanced.rs`. The orchestrator removes the variant from fh-core after you merge.
- Hook latency: add a test or example in fh-ledger that measures `handle_event` for 2000 non-verification events and prints p50/p95/p99 (the real process-level number is measured by Track F through the binary).
- Tests needing a redactor or scanner: write a tiny local test implementation; the real one is Track C's.

<!-- Derived from the Firsthand master build spec. Internal working document. -->

# Track B

**Track B: Verification engine and semantics** (branch `track/b-verify`, ~8 hours)
Owns: `crates/fh-claims`, `crates/fh-sem`.
Deliver: claim and probe execution per 8.2 (static probes against trees, ledger probe, command executor with gates, env scrub, process-tree kill using Job Objects/process groups, output caps), evaluator per 8.3, drift per 8.4, scope hashing, extractor per 8.6 with a fixture corpus, `fh-sem` per 8.5 (Rust, TS/JS, Python; Go last) with golden symbol fixtures and semantic diff.
Done when: AC-08 (static part), AC-09, AC-10; property tests (scope unchanged implies not affected; formatting-only change keeps `body_hash`; symbol move keeps identity); executor tests including timeout kill of a process tree on Windows and Unix, env scrub test, argv injection test, deny list test.

## Orchestrator notes (G0)

- Public API to implement: `CONTRACTS.md` sections 6 and 7. `fh-claims` must not depend on `fh-ledger`; probes read `&[LedgerEntry]`. Build test ledgers by hand (compute `hash` with `fh_core::canon::hash_canonical` over the entry with `hash` omitted; seq 0 `prev` is `B3::of_parts(&[b"firsthand.ledger/1", leg_id.as_bytes()])`).
- `derive_witnessed_claims` replaces hook-time claim creation (DEVIATIONS D4).
- `fh-git` `head_relation` and `diff_name_status` exist already. Bundles and worktrees are Track A's; for `--sandbox worktree` call `Repo::worktree_add_detached` (it returns NotYetImplemented until A merges; test that path behind `#[ignore]` with a note, and the orchestrator re-enables it at integration).
- The process-tree kill module is the only place `unsafe` may appear (narrow `#[allow(unsafe_code)]` with a justification comment). Prefer `windows-sys` Job Objects on Windows and process groups via `libc` on Unix.

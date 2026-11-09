<!-- Derived from the Firsthand master build spec. Internal working document. -->

# Track G

**Track G: HandoffBench** (branch `track/g-bench`; **G1 sub-track starts at hour 2.5** for scenarios/oracles/S0 to S3; S4 and runner finalize after F)
Owns: `crates/fh-bench`, `bench/`.
Deliver: all 20 scenarios and oracles, strategies S0 to S4, runner, metrics, report generator, perf harness, `bench-oracles-v1` tag flow (17.6), result JSON schema, `firsthand bench` integration.
Done when: AC-18 and the integrity rules in 17.6 are satisfied; the oracle hash CI check exists; results generated from a real run.

## Orchestrator notes (G0)

- G1 sub-track (now): author all 20 scenarios (spec 17.3) as deterministic Rust scripts in `crates/fh-bench` that build real temp git repos and emit `SessionArtifacts`; write `bench/scenarios/T01..T20/spec.json` and `oracle.json`; implement strategies S0 to S3 (S3 at two diligence levels: all claims checked, half checked); metrics per 17.5 with N/A never counted as 0; the strategy-visible-input grep test from 17.2; `bench/ORACLES.sha256` computed over every `oracle.json` in sorted path order (format: `<blake3 hex>  <path>` lines plus a final `total` line); an `oracles-hash` check function the CI can call through `cargo run -p fh-bench --example oracle_hash -- --check` (or similar, documented in `bench/README.md`).
- Do NOT implement S4 scoring now. S4 needs fh-flows (Track F) and is added only after the orchestrator tags `bench-oracles-v1`.
- `fh-bench` may depend on fh-core, fh-git (Phase 0 API), fh-testkit. It must not depend on crates owned by tracks still in progress.
- SessionArtifacts must contain everything a witness-based strategy needs later (simulated hook events as `fh_core::HookEvent` values, in order), so S4 can replay them through the real hook core.

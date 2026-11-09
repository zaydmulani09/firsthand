<!-- Derived from the Firsthand master build spec. Internal working document. -->

# Track C

**Track C: Packet, redaction, brief** (branch `track/c-packet`, ~8 hours)
Owns: `crates/fh-packet`, `crates/fh-redact`, `crates/fh-brief`, `spec/vectors/` generator (`xtask vectors` code lives in `xtask` but this track owns the vector content), `spec/*.md` edits for these areas.
Deliver: `.fhp` writer/reader (deterministic), manifest, signing, key management and trust store, verify per 6.4, hardening per 6.5 (a test per rule), inspect, redaction engine with rules in 15.2 and overlay, brief compiler per Section 11 with golden tests, bootstrap prompts, AGENTS.md block insert/remove.
Done when: AC-06, AC-07, AC-11, AC-19; byte-flip test over every byte of a sample packet (every flip detected or provably harmless padding-free), zip attack suite, vectors generated, redaction corpus (at least 60 positive and 60 negative cases) with idempotence property test, brief golden snapshots and the T11 fence tests.

## Orchestrator notes (G0)

- Public API to implement: `CONTRACTS.md` section 8.
- `StandardRedactor` implements both `fh_core::Redactor` and `fh_core::SecretScanner`.
- Vectors: put the generator in `crates/xtask/src/vectors.rs` and add a `vectors` task to `crates/xtask/src/main.rs` (you own only those lines). Output to `spec/vectors/`. Track H's TS SDK consumes them later, so document each file's format in `spec/vectors/README.md`.
- You may tighten `spec/PACKET.md`, `spec/BRIEF.md`, `spec/TRUST.md` to match what you build. Keep them passing `cargo xtask lint-copy`.

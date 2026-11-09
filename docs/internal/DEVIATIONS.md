# Deviations from the master spec

Each entry: what, evidence, smallest viable alternative taken.

## D1. New crate `fh-flows`

What: spec 4 has no orchestration library. The CLI, MCP server, dashboard and the
benchmark's S4 strategy all need `seal` and `receive`. Putting them in `fh-cli` creates a
dependency cycle (`fh-cli` depends on `fh-bench` for `firsthand bench`, and `fh-bench`
needs the flows for S4).

Alternative: `crates/fh-flows` owned by Track F. `fh-mcp`, `fh-ui` (at integration),
`fh-bench` and `fh-cli` depend on it.

## D2. ed25519-dalek 3.x instead of 2.x

What: spec 3.1 names `ed25519-dalek` v2. `cargo add` resolves 3.0.0 on 2026-11-08 (R19:
use current releases). Same algorithm and key format.

## D3. Hook-time SessionStart arming lives in Track A

What: spec 18.4 lists AC-14 under Track D "using Track A's hook core". A and D run in
parallel, so D cannot call A's code. `handle_event` (A) implements SessionStart arming
and leg linking; D parses harness JSON into `HookEvent`; F proves AC-14 end to end.

## D4. Witnessed claims are derived at seal time

What: spec 2.3 (1) creates claims after witnessed verification commands. Writing claim
entries from the hook fast path would make `fh-ledger` depend on `fh-claims`. Instead
`fh_claims::derive_witnessed_claims` projects the ledger at seal time. Same claims, same
tier, no extra hook latency.

## D5. Copy lint treats "harness" as banned only as a verb

What: Appendix E bans "harness"; the product spec uses "harness" as the noun for the agent
runtime that emits hook events ("harness hooks"). The lint bans the verb form only.

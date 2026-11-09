<!-- Derived from the Firsthand master build spec. Internal working document. -->

# Track D

**Track D: Agent adapters and hooks** (branch `track/d-agents`, ~8 hours)
Owns: `crates/fh-adapters`, `crates/fh-fakeagent`, adapter fixtures in `crates/fh-adapters/tests/fixtures`.
Deliver: trait and implementations for claude-code, codex, gemini-cli, opencode, generic per Section 10 with `detect`, `discover_sessions`, `ingest`, `hook_plan` (+ apply with diff, backup, tag, idempotence, uninstall), `parse_hook_event`, `mcp_install_plan`, `launch_spec`; `fh-fakeagent` binary that emits realistic hook invocations and writes session logs in each agent's documented format for tests; verification report for `docs/adapters.md` with honest matrix (10.3).
Done when: AC-13, AC-14 (SessionStart arming logic, using Track A's hook core), adapter fixture tests for every agent (normal, tool pairs, injection string, fake secret, malformed trailing line, huge line), `init`/`uninstall` plans byte-exact round trip on temp settings files, Windows path handling tests (`projects` encoded cwd decoding).

## Orchestrator notes (G0)

- Public API to implement: `CONTRACTS.md` section 9. You own `crates/fh-adapters` and `crates/fh-fakeagent`.
- None of claude, codex, gemini, opencode is on PATH on this machine. `~/.claude/projects` exists (written by the Claude desktop app); you may read it to learn the format but never copy content (R4). All other agents: synthetic-by-spec fixtures from their public docs (you may fetch docs pages with WebFetch or the web-fetch agent if available to you; otherwise work from the spec text and say so in the matrix).
- Hook stdin JSON parsing is yours (`parse_hook_event` -> `fh_core::HookEvent`). Arming and ledger writes are Track A's (DEVIATIONS D3).
- `fh-fakeagent`: a binary that (a) writes session logs in each agent's documented format into a given directory, and (b) emits hook invocations by spawning a given firsthand binary path with harness-shaped JSON on stdin. Track F's e2e tests drive it.
- Write the honest compatibility matrix to `docs/adapters.md` (create it; it is yours).

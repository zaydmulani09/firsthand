<!-- Derived from the Firsthand master build spec. Internal working document. -->

# Track E

**Track E: Dashboard and UI server** (branch `track/e-ui`, ~9 hours)
Owns: `packages/dashboard`, `packages/schema-types` consumption, `crates/fh-ui`, `docs/assets/` screenshots tooling.
Deliver: all ten views in 14.3, design system per 14.4, API per 14.2 (handlers call library functions through frozen traits; until Wave 1 merges they work against a **fixture-backed implementation of the traits used only in `fh-ui` tests**, replaced by the real implementations at integration), SSE, token/Host/Origin/CSP security, embedded assets via `rust-embed`, static export mode, Playwright e2e and axe tests, theme toggle, keyboard navigation.
Done when: AC-15, AC-16; screenshots captured by Playwright from the real server running on fixture-generated real packets (for docs); bundle budget met; zero console errors; works with no network.

## Orchestrator notes (G0)

- Public API: `CONTRACTS.md` section 10. You own `packages/dashboard`, `packages/schema-types`, `crates/fh-ui`, `docs/assets/`, and the `build-ui` task (add it in a new file `crates/xtask/src/ui.rs` plus the dispatch line in `main.rs`).
- Create the root `package.json` with npm workspaces `["packages/*"]` and commit `package-lock.json`. Track H adds `packages/sdk` later.
- Dev fixtures: `cargo xtask gen-fixtures` writes `packages/dashboard/fixtures/` from fh-core types. Extend fixtures inside your own fh-ui test DataSource as needed.
- Generate `packages/schema-types` from `schema/*.json` with `json-schema-to-typescript` and add `gen-ts` to xtask (same `ui.rs` file).
- Playwright: headless only, never a visible browser, never Claude in Chrome or computer use. Install browsers with `npx playwright install chromium`.
- Screenshots for docs come later from real packets; for now make the screenshot script work against your fixture-backed server.

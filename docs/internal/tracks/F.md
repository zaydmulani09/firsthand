<!-- Derived from the Firsthand master build spec. Internal working document. -->

# Track F

**Track F: CLI, MCP, flows** (branch `track/f-cli`, ~7 hours, starts when A, B, C merged; D partially)
Owns: `crates/fh-cli`, `crates/fh-mcp`.
Deliver: every command in 12.1 wired to library APIs with human and JSON output, exit codes, `doctor` live self-test, `init/uninstall` using D's plans, `run`, `handoff`, `receive` (including armed injection and clipboard on Windows `clip.exe`, macOS `pbcopy`, Linux `wl-copy`/`xclip`/`xsel` if present, otherwise print), `verify`, `drift`, `diff`, `restore`, `replay`, `report --html`, `search`, `import md`, completions; MCP server per Section 13 with an interoperability test using the official TS MCP client; `cargo xtask gen-docs` for `docs/cli.md`.
Done when: AC-01, AC-02, AC-12, J1 to J4 scripted end to end in `crates/fh-cli/tests/e2e_*.rs` using `fh-fakeagent`, snapshot-tested help output, exit code tests.

# Contract amendment requests

Append requests here. Format:

```
## <track>-<n>: <short title>
Requested by: track X
What: ...
Why: ...
Proposed signature or change: ...
Status: open | accepted (commit) | rejected (reason)
```

## D-1: hook stdout formatting per agent
Requested by: track D
What: Gemini CLI parses hook stdout as JSON; plain bootstrap text breaks it.
Why: AC-14 SessionStart injection for gemini-cli.
Proposed signature or change: the `firsthand hook` entry point (A/F) prints `fh_adapters::hook_stdout(agent, kind, &outcome.stdout)` instead of the raw text. Identity for every other agent.
Status: open

## D-2: ingest output is unredacted
Requested by: track D
What: `AgentAdapter::ingest` returns raw text (fh-adapters cannot depend on fh-redact).
Why: spec 10.1 says messages are redacted; the frozen signature has no redactor.
Proposed signature or change: F calls `fh_adapters::redact_session(&mut s, &redactor)` right after `ingest`, before anything is stored or extracted.
Status: open

## D-3: init plumbing for F
Requested by: track D
What: (a) apply each agent's hook plan before computing its MCP plan (Gemini uses one file for both; a stale plan is refused); (b) add `*.firsthand-*.bak` and `.claude/settings.local.json` to `.git/info/exclude`; (c) Gemini repo scope: if `.gemini/settings.json` is tracked, prefer `--scope user` or show the manual step.
Why: fh-adapters has no git access and plans are pure.
Proposed signature or change: none in D; F's `init`.
Status: open

## D-4: FileEdit removal convention, extra public API
Requested by: track D
What: `FileEdit` with `before: Some(_)` and empty `after` means "delete the file" (`FileEdit::is_removal`). Additive API: `get_with`, `uninstall_plan_with`, `HostEnv::{from_process, with_home}`, `IngestOpts::default`, `encode_project_dir`, `plugin_source`, `OPENCODE_PLUGIN_PATH`, `hook_stdout`, `redact_session`, concrete adapter structs.
Why: uninstall must remove files init created; tests need fake homes.
Proposed signature or change: as listed, no change to frozen items.
Status: open

## D-5: OpenCode plugin directory
Requested by: track D
What: plugin written to `.opencode/plugins/firsthand.ts`, not `.opencode/plugin/` (spec 10.1).
Why: current OpenCode docs (fetched 2026-11-09) only document auto-loading from `.opencode/plugins/`.
Proposed signature or change: spec text update.
Status: open

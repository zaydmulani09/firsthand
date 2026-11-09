# Agent adapters

Firsthand talks to each coding agent through an adapter in `crates/fh-adapters`. An
adapter reads that agent's session logs (read-only, as hearsay), plans hook and MCP
config edits, turns the agent's hook JSON into a normalized event, and describes a
launch command. It never sends a prompt and never runs the agent beyond `--version`.

## Compatibility matrix

Recorded 2026-11-09 on Windows 11 (MSVC toolchain). None of `claude`, `codex`,
`gemini` or `opencode` is installed on the build machine, so every row below was
tested against synthetic fixtures only. No hook was observed firing in a live agent.

| Agent | Version tested | Ingest | Hooks | MCP config generated | Launch |
|---|---|---|---|---|---|
| Claude Code (`claude-code`) | none installed | fixture | fixture | yes, `.mcp.json` (round trip tested, not loaded by a live agent) | documented-only |
| Codex CLI (`codex`) | none installed | fixture | fixture | yes, `.codex/config.toml` (round trip tested, not loaded by a live agent) | documented-only |
| Gemini CLI (`gemini-cli`) | none installed | fixture | fixture | yes, `.gemini/settings.json` (round trip tested, not loaded by a live agent) | documented-only |
| OpenCode (`opencode`) | none installed | fixture (SQLite and JSON storage) | fixture (plugin text only, never loaded by OpenCode) | yes, `opencode.json` (round trip tested, not loaded by a live agent) | documented-only |
| Generic (`generic`) | n/a | none | none | manual step printed | none |

"Fixture" means the format was taken from the agent's public documentation (fetched
2026-11-09) or, for Claude Code, from read-only inspection of the structure of local
transcripts. Fixture content is synthetic: no text, path or identifier was copied
from real sessions.

## What each adapter does

### Claude Code

- Sessions: `~/.claude/projects/<encoded-cwd>/<session-id>.jsonl` (or under
  `$CLAUDE_CONFIG_DIR`). The directory name replaces every character other than an
  ASCII letter or digit with `-`, so `C:\Users\dev\My Repo.v2` becomes
  `C--Users-dev-My-Repo-v2`. The mapping is lossy, so discovery treats the name as a
  prefix filter and then confirms each session by its recorded `cwd`.
- Ingest: `user` and `assistant` lines; `text`, `tool_use` and `tool_result` blocks.
  `thinking` and `redacted_thinking` blocks, `isMeta` lines and sidechain lines are
  skipped. Compaction summaries are kept as system messages. Subagent transcripts in
  `<session>/subagents/*.jsonl` become one `subagent` tool event each.
- Hooks: `.claude/settings.local.json` (repo) or `~/.claude/settings.json` (user):
  `SessionStart` (matcher `startup|resume|clear|compact`), `PreToolUse` (`Bash`),
  `PostToolUse` (`Bash|Write|Edit|MultiEdit|NotebookEdit`), `PostToolUseFailure`
  (`Bash`), `Stop`, `SubagentStop`, `PreCompact`, `SessionEnd`.
- Exit codes: Claude Code reports no exit code for a successful Bash call. Firsthand
  records 0 for `PostToolUse` (which fires only on success, unless `interrupted`) and
  parses the `Exit code N` first line of `PostToolUseFailure.error`. Verify on a live
  install before relying on it.
- Restart open sessions after `init`: hooks load at session start.

### Codex CLI

- Sessions: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` (or under `$CODEX_HOME`),
  matched by the `cwd` of the `session_meta` line. `reasoning` items are skipped;
  `event_msg` lines are skipped because they repeat response items.
- Hooks: `.codex/hooks.json` with the same nested shape as Claude Code. Codex runs
  project hooks only after you trust them: open Codex in the repo, run `/hooks`, and
  trust the firsthand entries. Trust follows the hook hash, so trust them again after
  `firsthand init` changes them. This is a manual consent step, so `witness_bash` is
  `Partial`.
- Legacy `notify` (JSON as the last argument) is parsed as a `Stop` event; `init` does
  not install it.
- MCP: a managed block in `.codex/config.toml` between `# firsthand:begin` and
  `# firsthand:end`. An existing `[mcp_servers.firsthand]` outside the block is left
  alone and reported.

### Gemini CLI

- Sessions: `~/.gemini/tmp/<dir>/chats/*.json`, where `<dir>` is the SHA-256 of the
  project root, a slug listed in `~/.gemini/projects.json`, or a directory whose
  `.project_root` file names the root. `thoughts` are never ingested.
- Hooks: `.gemini/settings.json`: `SessionStart`, `BeforeTool`
  (`run_shell_command`), `AfterTool` (`run_shell_command|write_file|replace`),
  `AfterAgent`, `PreCompress`, `SessionEnd`. Gemini parses hook stdout as JSON, so the
  SessionStart bootstrap is wrapped in `hookSpecificOutput.additionalContext`
  (`fh_adapters::hook_stdout`). Gemini sends no tool use id, so Pre and Post events
  are not paired by id.
- `.gemini/settings.json` may be tracked by git. If it is, use `--scope user`.
- Hooks and MCP share that file: apply the hook plan before computing the MCP plan.

### OpenCode

- Sessions: `opencode.db` opened with `mode=ro` (tables `session`, `message`, `part`;
  an unknown schema is refused with the columns found), plus the older JSON tree under
  `storage/`. Data directories probed: `$XDG_DATA_HOME/opencode`,
  `~/.local/share/opencode`, `%LOCALAPPDATA%\opencode`, `%APPDATA%\opencode`.
  `reasoning` parts are never ingested.
- Hooks: a generated plugin at `.opencode/plugins/firsthand.ts` that forwards
  `tool.execute.before`, `tool.execute.after`, `session.created`, `session.idle`,
  `session.compacted` and `file.edited` to `firsthand hook opencode <event>` with JSON
  on stdin. The product spec named `.opencode/plugin/`; the current OpenCode docs name
  `.opencode/plugins/`, so Firsthand writes there. The docs also describe a newer
  `Plugin.define` API; the generated file uses the older function export, which has
  not been checked against any OpenCode release.
- No SessionStart injection: deliver the brief through the clipboard prompt or the
  AGENTS.md block.

### Generic

Any agent that can run a shell command or speak MCP. Use `firsthand run -- <cmd>`,
`firsthand note`, `firsthand claim`, the MCP server, and `firsthand import md`.

## Config edits

- Default scope is repo-local. User files under the home directory are planned only
  with an explicit user scope.
- Every injected hook command is `<firsthand> hook <agent> <Event>` and contains the
  literal `firsthand hook`. Uninstall removes exactly those entries, the matcher groups
  that held only them, and an MCP server named `firsthand` whose command ends in
  `mcp serve`.
- Edits are spliced into the existing text. Formatting, comments and key order of the
  rest of the file stay as they were, and install followed by uninstall restores the
  original bytes. One known exception: an event list or object that already existed
  but was empty (`"Stop": []`, `"mcpServers": {}`) is removed together with the
  Firsthand entry placed in it.
- Applying a plan checks that each file still matches what the plan was computed
  from, writes `<file>.firsthand-<timestamp>.bak`, then replaces the file atomically.
  Running `init` twice changes nothing.

## Not verified

- No live hook firing for any agent (no agent CLI installed).
- Launch commands (`claude "<prompt>"`, `codex "<prompt>"`, `gemini -i "<prompt>"`,
  `opencode --prompt "<prompt>"`) come from documentation and are marked
  `verified: false`.
- Codex `tool_response` shape for Bash in hooks is not documented; Firsthand reads
  an object with exit code fields or text in the `Exit code: N ... Output:` form.

## Fake agent

`crates/fh-fakeagent` writes session logs in each format above into a fake home
directory and emits hook invocations by spawning a `firsthand` binary with the
agent's JSON on stdin. Its tests check that the real adapters read both back.

```text
fh-fakeagent session --agent codex --home <dir> --cwd <repo>
fh-fakeagent script --firsthand <path-to-firsthand> --agent claude-code --cwd <repo> --pairs 100
```

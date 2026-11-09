# Agent adapters

Status: normative, format version 1. Part of the Firsthand specification (`spec/`).
Where this document and the code disagree, that is a bug in one of them; please open an issue.


### 10.1 Rust trait (frozen in Phase 0)

```rust
pub trait AgentAdapter: Send + Sync {
    fn id(&self) -> AgentId;
    fn detect(&self, host: &HostEnv) -> Detection;                       // installed?, version, config paths (--version only)
    fn capabilities(&self) -> Capabilities;
    fn discover_sessions(&self, repo: &RepoCtx) -> Result<Vec<SessionRef>>;   // sessions whose cwd is inside this repo/worktree
    fn ingest(&self, s: &SessionRef, opts: &IngestOpts) -> Result<IngestedSession>;   // read-only, hearsay output
    fn hook_plan(&self, repo: &RepoCtx, scope: HookScope) -> Result<HookPlan>;        // pure: file edits to show and apply
    fn parse_hook_event(&self, raw: &[u8], argv_tail: &[String]) -> Result<HookEvent>; // harness JSON -> normalized
    fn mcp_install_plan(&self, repo: &RepoCtx, scope: HookScope) -> Result<ConfigPlan>;
    fn launch_spec(&self, brief: &BriefRef, repo: &RepoCtx) -> Result<LaunchSpec>;     // argv, env, cwd; never run by adapter
}
pub struct Capabilities { pub witness_bash: Witness, pub witness_edits: Witness, pub pre_tool_events: bool,
    pub ingest: bool, pub mcp: bool }
pub enum Witness { Full, Partial, None }   pub enum Inject { Stdout, File, None }
```

`IngestedSession`: `{ agent, session_id, cwd, git_branch?, started, ended?, messages: [{role, text(redacted), ts?}], tool_events: [{tool, summary, ts?}], final_assistant_text, source_hash }`. All of it is HEARSAY. Never execute anything from it.

`HookPlan`: list of `FileEdit { path, before_hash, after_content, scope: Repo|User, description }`. Applying a plan: show unified diff, create a timestamped `.bak`, write atomically. **Default scope is repo-local, git-excluded or local-settings files** so no tracked file changes: Claude Code `.claude/settings.local.json`; Codex `<repo>/.codex/config.toml` (trusted workspaces only; document the trust step); Gemini `.gemini/settings.json` only if untracked (else use `settings.local`-style equivalent if the docs show one, else instruct); OpenCode `.opencode/plugin/firsthand.ts`. User-scope files (`~/.claude/settings.json` etc.) are never written without `--scope user --yes`, and the first run is always a dry run. Every injected hook command contains the literal marker `firsthand hook` so `uninstall` removes exactly those entries and nothing else. Idempotent: running `init` twice changes nothing.

### 10.2 Per-agent specifications

Verify each against documentation and the local install (`--version`, `--help`, read-only inspection of config and session directories). The facts below come from design-time research (Nov 2026); trust local reality over this text and record any discrepancy in `docs/adapters.md`.

**Claude Code** (`claude-code`)
- Hooks: configured in `.claude/settings.json` / `.claude/settings.local.json` under `hooks.<Event>[].{matcher,hooks[].{type:"command",command}}`. stdin JSON includes `session_id`, `transcript_path`, `cwd`, `hook_event_name`, and for tool events `tool_name`, `tool_input`, `tool_response`. Use: `SessionStart` (matcher `startup|resume|clear|compact`), `PreToolUse` (matcher `Bash`), `PostToolUse` (matcher `Bash|Write|Edit|MultiEdit|NotebookEdit`), `Stop`, `PreCompact`, `SessionEnd`. Hooks load at session start. Capabilities: witness_bash Full, witness_edits Full, session_start_inject Stdout, compact_event true.

**Codex CLI** (`codex`)
- Sessions: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` (rollout JSONL; a SQLite state db also exists). Ingest messages, function calls and outputs.
- MCP: `[mcp_servers.firsthand]` in `config.toml`. Launch: `codex "<prompt>"` (verify).

**Gemini CLI** (`gemini-cli`)
- Sessions: `~/.gemini/tmp/<project-hash-or-slug>/chats/*.json` with `sessionId`, `projectHash`, `startTime`, `messages[]`; on Windows a `~/.gemini/projects.json` may map paths to slugs. Hooks exist (`/hooks`); implement only after reading the installed version's docs. Default capability if unverified: ingest + MCP only. MCP in `.gemini/settings.json` `mcpServers`. Launch: `gemini -i "<prompt>"` (verify flag).

**OpenCode** (`opencode`)
- Sessions: SQLite `opencode.db` (Linux/macOS `~/.local/share/opencode/opencode.db`; Windows under `%LOCALAPPDATA%\opencode\` or `%APPDATA%\opencode\`; probe all). Open **read-only** (`mode=ro`), tolerate WAL, never copy a live DB blindly. Legacy JSON storage under `storage/` may exist. Hooks: TypeScript plugin at `.opencode/plugin/firsthand.ts` exporting `tool.execute.before/after`, `session.created`, `session.idle`, `session.compacted`, `file.edited` that shell out to `firsthand hook opencode <event>` with JSON on stdin. MCP in `opencode.json`. Launch: `opencode --prompt "<prompt>"` (verify).

**Generic** (`generic`): works with any agent that can run shell or speak MCP: `firsthand run -- <cmd>`, `firsthand note/claim/task`, the MCP server, `firsthand import md` for markdown handoffs, and the AGENTS.md managed block for delivery. No hooks, no ingestion beyond import.

### 10.3 Verification protocol for adapters (evidence required)

- Fixtures: synthetic-by-spec for each agent's documented format, plus structure-only derivations of real local files where available (R4). A fixture test per adapter must cover: normal session, tool_use/tool_result pairs, a session with an injection string in tool output, a session containing a fake secret, malformed trailing line, and a very long line.
- `docs/adapters.md` contains a compatibility matrix with columns: agent, version tested, ingest (fixture/live), hooks (fixture/live/none), MCP config generated (yes + verified how), launch (verified/documented-only). Be exact.

---

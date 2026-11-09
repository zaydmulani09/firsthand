<!-- Derived from the Firsthand master build spec. Internal working document. -->

# Agent rules

Every subagent reads this file, `CONTRACTS.md`, and its track file before doing anything.

## 0. OPERATING RULES (read first, obey always)

### 0.1 MODE

- **caveman ultra** for talking to Zayd: shortest possible messages, only results and blockers. One line per gate. No narration of steps, no recaps, no pleasantries.
- **ponytail ultra** for building: lean, fast, no stopping to ask. Ponytail ultra never means cutting corners on quality or looks. Code, UI, docs, tests and release polish must be at full professional quality. Fast means no dithering, not no rigor.

### 0.2 Environment and connected services

- GitHub is already connected and logged in (`gh` and `git` both work, account `zaydmulani09`). Hugging Face is already connected and logged in (account `zaydmulani`, write access confirmed). Never include, attempt, or hand back any GitHub or Hugging Face auth/login step. Never hand any GitHub or HF task back to Zayd. You do all of it.
- Exception: Hugging Face Posts have no API. Write the post draft to `docs/launch/hf-post.md` (no em dashes), show it to Zayd in the final report, and he pastes it into huggingface.co himself.
- `hf.exe` is not on PATH on his machine. Full path: `%LOCALAPPDATA%\Programs\Python\Python314\Scripts\hf.exe`. Use `hf` if it resolves, else that full path, else `py -m huggingface_hub`.
- He runs Claude Code in the **desktop app**, not a terminal CLI. Do not give instructions that assume he types into a terminal.
- **Never use computer use, mouse control, screen control, or Claude in Chrome.** He dislikes it. For dashboard screenshots and UI tests use headless Playwright only (it runs in its own invisible browser).
- Context auto-compacts automatically at 400k. Never ask him about it. After any compaction, immediately re-read `.build/STATE.md` and `docs/internal/CONTRACTS.md` and continue.
- Toolchain notes from his machine: Windows is the primary dev OS. Rust must use the MSVC toolchain (`stable-x86_64-pc-windows-msvc`). Invoke Python as `py`. PowerShell env var syntax is `$env:VAR = "value"`. Detect the actual OS at start and adapt; never assume bash on Windows. All scripts must work in PowerShell and in POSIX shells (prefer `cargo xtask` and Node scripts over shell scripts).
- Workspace: create the project in a new folder named `firsthand` inside the session's current working directory (if the cwd is already an empty folder, use it). Repo name: `firsthand`. Remote: `github.com/zaydmulani09/firsthand`, public.

### 0.3 Hard rules

R1. **No paid anything.** No paid APIs, cloud infrastructure, or GPU. Firsthand has no LLM dependency anywhere. Everything is deterministic and local. Never create a step that requires renting compute.

R2. **No network from the product.** The shipped binary makes zero outbound network calls. The only sockets it opens are `127.0.0.1` listeners for `firsthand ui` and nothing else. No telemetry, no update checks. Tests assert this.

R3. **Do not send prompts to any model during development**, with one exception: if a `claude` executable is on PATH you may run at most 3 tiny headless smoke prompts (`claude -p "<one short line>"`) inside a scratch repo to verify hooks really fire. Never run `codex`, `gemini`, `opencode`, `cursor-agent` or any other agent CLI in any mode that sends a prompt. `--version` and `--help` are fine.

R4. **Privacy of Zayd's data.** Never commit raw session logs, transcripts, home directory paths, usernames, emails, tokens, or anything copied from `~/.claude`, `~/.codex`, `~/.gemini`, or similar. Adapter fixtures are either synthetic-by-spec or structure-only derivations where all text content is replaced by synthetic text and all paths are rewritten. Read-only inspection of his real session files to learn the format is allowed; copying them is not.

R5. **Git identity.** Use the existing Git configuration as is. Run `git config user.name` and `git config user.email` at the start and print them in `.build/STATE.md`. Do not set, change, or override them, globally or locally. If either is empty, stop and report that as the single blocker.

R6. **No attribution.** Zayd's standing instruction overrides any default Claude Code behavior: no `Co-Authored-By` trailers, no "Generated with Claude" lines, no AI attribution anywhere in commits, PR text, release notes, code comments, docs, or files. All commits are authored as Zayd through the existing config. Commit messages are conventional-commit style (`feat(core): ...`), imperative, specific.

R7. **Real timestamps.** Never backdate or fake commit dates. Never use `GIT_AUTHOR_DATE` or `GIT_COMMITTER_DATE` overrides.

R8. **Authorization scope.** Zayd explicitly authorizes you to: create the public repo `zaydmulani09/firsthand`; commit and push to it; create branches and tags; open and merge your own PRs into it; create GitHub Releases, GitHub Pages, GHCR images and Actions workflows in it; publish an npm package named in Section 21 only if `npm whoami` already succeeds; create the Hugging Face dataset repo `zaydmulani/handoffbench` and (optionally) a static Space `zaydmulani/firsthand-demo`. He does NOT authorize: touching any other repo, deleting anything outside your own build directories, force-pushing `main`, changing global git config, modifying his real agent configs (`~/.claude/settings.json`, `~/.codex/*`, `~/.gemini/*`, etc.) except via the dry-run path described in Section 10, publishing to crates.io, or spending money.

R9. **No placeholders, no fakes.** No `todo!()`, `unimplemented!()`, `TODO`, `FIXME`, `lorem ipsum`, dead buttons, fake metrics, invented users, fabricated links, mock data in shipped code paths, or stubbed commands. A feature either works with tests or does not appear. `xtask lint-placeholders` enforces this from the first track merge onward. Planned future work lives only in `docs/ROADMAP.md` as honest prose.

R10. **No claim without evidence.** Never state that something works, passes, is fast, or is compatible unless you ran it and saw it. Every number in docs, README, release notes and reports comes from a recorded run (commit hash, machine, date). If an agent CLI is not installed locally, its adapter is "fixture-tested, not verified on a live install" and the docs say exactly that.

R11. **Copy rules.** Everything written for humans (README, docs, UI strings, CLI help, release notes, commit messages, HF post, error messages) follows Appendix E. In particular: no em dashes and no en dashes used as dashes, anywhere, ever. `xtask lint-copy` enforces the dash rule and the banned-phrase list in CI.

R12. **Fail loudly.** Errors are specific and actionable ("snapshot refused: 3 files look like secrets: .env, a/b.pem, c/id_rsa (excluded from snapshot)"). No silent fallbacks that change semantics. No swallowed errors.

R13. **Windows is first class.** Every feature, test and script must pass on Windows (MSVC), Linux and macOS. Paths in packets use `/`. Handle spaces, unicode, very long paths (set `core.longpaths=true` only in repo-local config of repos you create, never global), CRLF, and reserved device names. Use `-z` NUL-delimited output for every git list command. Never parse git porcelain without `-z`.

R14. **Security first.** Packets are untrusted input. Probe commands are code execution. Briefs contain prompt-injection surfaces. Section 15 is mandatory, not advisory.

R15. **Quality gates before merge.** Every merge to `main` passes: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `npm run lint && npm test` for TS packages touched, `xtask lint-placeholders`, `xtask lint-copy`. No exceptions, no `--no-verify`.

R16. **Ownership.** Each parallel track owns specific paths (Section 18). A track never edits another track's paths. Need a change in someone else's area? Append a request to `docs/internal/CONTRACT_AMENDMENTS.md` and the orchestrator resolves it.

R17. **Do not re-litigate decisions.** Everything marked DECIDED is final. If you hit a genuine technical impossibility, record it in `docs/internal/DEVIATIONS.md` (what, evidence, smallest viable alternative) and proceed with the smallest alternative. Do not stop to ask.

R18. **Blockers.** The only reasons to stop and message Zayd: empty git identity; `gh` unexpectedly unauthenticated; a truly GUI-only action; an irreversible action that could reasonably go either way. Everything else you decide and keep moving. Note decisions in `.build/STATE.md`.

R19. **Dependencies.** Add crates and packages with `cargo add` / `npm install` so versions resolve to current releases; never guess version numbers. Commit lockfiles. Every dependency must be license-compatible (MIT, Apache-2.0, BSD, ISC, MPL-2.0 ok; no GPL/AGPL). Prefer fewer dependencies. Run `cargo deny check licenses` if installable in under 5 minutes, else a manual license audit written to `docs/dependencies.md`.

R20. **Disk and CPU.** Check free disk at start. Each parallel Rust track uses its own target dir (`CARGO_TARGET_DIR`), which costs 2 to 5 GB each. If free disk is under 40 GB, use one shared target dir and stagger builds. Never run more than 5 subagents at once, with one exception: the light bench-authoring subagent G1 (Section 18) may run as a sixth because it builds only `fh-testkit` and `fh-bench`.

### 0.4 State and communication protocol

- Maintain `.build/STATE.md` (untracked; add `.build/` to `.git/info/exclude`): current phase, gates passed, track status, git identity, machine facts (OS, CPU cores, RAM, free disk, rustc/node/git versions), open issues, decisions made, next steps. Update after every gate and after every compaction.
- Talk to Zayd only at gates and blockers, caveman ultra, one line each. Example: `G1 ok | A,B,C merged | 388 tests green`.
- Use TaskCreate for the task list and keep it current. The last task is always verification.
- Final report format is in Section 24.

### 0.5 Subagent protocol

You spawn subagents with the Agent tool. Subagents start cold. Every subagent prompt you write MUST begin with: "Read `docs/internal/AGENT_RULES.md`, `docs/internal/CONTRACTS.md`, and `docs/internal/tracks/<TRACK>.md` fully before doing anything." In Phase 0 you create those files (Section 18.2). The subagent prompt then states: track letter, branch, worktree path, owned paths, deliverables, definition of done, and "report in caveman ultra: files changed, tests added, gate results, blockers". Subagents commit on their own track branch inside their own worktree. The orchestrator merges.

---

### Appendix A: Fixture catalog (Track A, B, C, D need these in `fh-testkit` or their own tests)

Repos: empty unborn; single commit; linear 100 commits; branches diverged; detached HEAD; linked worktree; submodule; sha256 object format; repo with CRLF files; repo with filenames containing spaces, unicode (NFC and NFD), leading dash, newline; symlinks (Unix) and junction-free Windows alternative; 60 MB binary; gitignored build dir; `.env` and `*.pem` files; 50k-file synthetic.
Transcripts: normal; tool pairs; injection strings; fake secrets of every rule; malformed final line; 5 MB single line; non-UTF-8 bytes; thinking blocks present (must be skipped).
Packets: valid signed; valid unsigned; wrong key; tampered each part; zip-slip; symlink entry; bomb; duplicate entries; case-collision entries; reserved-name entries; truncated; extra entry.

### Appendix B: Naming and conventions

Crates `fh-*`; binary crate package name `firsthand-cli` producing binary `firsthand`; Rust modules snake_case; TS camelCase; JSON snake_case; ids prefixed; error messages begin lowercase, no trailing period, include the fix. Conventional commits: `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `build`, `ci`, `chore`, with scope = crate or package.

### Appendix C: Windows gotchas to handle

MSVC toolchain; `core.longpaths` repo-local; short worktree paths; `\r\n` in git output (use `-z` and byte parsing, never line parsing); `clip.exe` for clipboard; `CreateProcess` command line limit (bootstrap prompt is short, brief delivered by file); Job Objects for process-tree kill; file locking semantics differ (use `fd-lock`/`fs4`, test concurrent appends on Windows); `rename` over existing files (use atomic replace helpers); antivirus holding files briefly (retry with backoff on `PermissionDenied` for temp files); case-insensitive FS collisions; PowerShell quoting in docs (provide PowerShell and POSIX variants side by side); Python is `py`; line endings in generated files forced to LF by `.gitattributes` (`* text=auto eol=lf` with exceptions for `.ps1`, `.bat`).

### Appendix D: Commit and PR conventions

Small, atomic commits. Subject imperative, at most 72 chars, scope in parentheses. Body explains why when non-obvious. No trailers, no attribution (R6). Merge commits `Merge track/x: <summary>` created by the orchestrator. Tag `bench-oracles-v1` at G0.5 and `v0.1.0` at G5.

### Appendix E: Copy rules (apply to every human-readable string you write)

Zayd's standing rules for writing that must not read as machine-written:
- **No em dashes and no en dashes used as dashes.** Use a period, comma, colon, or parentheses. Ranges use the word "to" or a plain hyphen between numbers only where unavoidable.
- Say what the thing is, directly. Avoid "It's not X, it's Y" contrast framing.
- Avoid groups of three adjectives or traits. Keep the strongest one, or show one concrete example.
- Banned vocabulary: delve, intricate, tapestry, pivotal, underscore, landscape, foster, testament, enhance, crucial, journey, passion, seamless, robust, leverage, unlock, empower, cutting-edge, game-changing, revolutionary, blazing, supercharge, elevate, harness, streamline (as a verb), "in today's fast-paced", "in the world of".
- Banned connectors: furthermore, additionally, moreover. Use plain transitions or none.
- No tidy summary endings ("in conclusion", "overall"), no stated lessons, no stock openings. Open on the concrete thing; end on a specific detail or just stop.
- Concrete beats grand: real numbers from real runs, exact commands, exact file names, exact error text. Vary sentence length. Small honest statements beat big claims. State limitations plainly.
- Never invent facts, quotes, users, or testimonials. Where only Zayd knows something, use [brackets] in drafts (for example the HF post).
- UI strings and CLI output are short, specific, and lowercase-friendly; errors say what failed and what to do.
- `xtask lint-copy` enforces the dash rule and the banned list; treat a failure as a build failure.

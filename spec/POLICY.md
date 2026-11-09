# Policy

Status: normative, format version 1. Part of the Firsthand specification (`spec/`).
Where this document and the code disagree, that is a bug in one of them; please open an issue.

### 5.6 Policy (`policy.toml`, user-local, authoritative)

```toml

### 8.1 Snapshot algorithm (`fh_git::snapshot_dirty`)

Never touches the real index, HEAD, branches, or working files.

1. Resolve index path with `git rev-parse --git-path index` (correct for linked worktrees). Copy it to a temp file inside the git common dir. Copying keeps the stat cache so `add -A` only hashes changed and new files. Unborn repo: start from an empty temp index.
2. Pre-scan candidates: `git ls-files -z --others --exclude-standard` plus `git diff --name-only -z HEAD`. Apply `snapshot.exclude_globs` and `max_file_bytes`. Run the secret content scanner (`fh-redact`) on the first 64 KiB of each untracked or modified text candidate. Matches are excluded and recorded in `DirtySummary.omitted` with reason `SecretPattern` or `SecretContent`. Print a loud warning naming the files. There is no flag to include suspected secrets.
3. With `GIT_INDEX_FILE=<temp>`: `git add -A --ignore-errors -- .` plus `:(exclude)` pathspecs for every omitted path. Then `git write-tree`.
4. `git commit-tree <tree> -p <HEAD if any> -m "firsthand snapshot <id>"` with fixed identity set only through env vars for that one call (`GIT_AUTHOR_NAME=firsthand`, `GIT_AUTHOR_EMAIL=firsthand@localhost`, committer same). This never reads or writes the user's git config and creates no history on any branch.
5. `git update-ref refs/firsthand/snapshots/<id> <commit>` (or `refs/firsthand/auto/<n>` rotating). Delete the temp index. Return `SnapshotRef` plus `DirtySummary`.
6. `fh_git::tree_only()` variant: steps 1 to 3 only, returns the tree oid without a commit or ref. Used to stamp verification commands cheaply.

Acceptance: before/after comparison of `git status --porcelain=v2 -z`, the real index file checksum, HEAD, and every working file hash is identical (test). Submodules are recorded as gitlinks, never recursed. Gitignored files are never captured.

Restore (`fh_git::restore`): `--to <dir>` uses `git worktree add --detach <dir> <commit>`. `--apply` always takes a `PreRestore` snapshot of the current state first (so nothing current is lost), then runs `git checkout <commit> -- .`; `--exact` also removes tracked and previously-snapshotted files absent from the target snapshot (never deletes files that are neither in HEAD nor in either snapshot). Fully reversible by restoring the pre-restore snapshot.

Bundle: `git bundle create <tmp> <snapshot_commit> ^<base_head>` (thin). Materialize on the receiver with `git bundle verify` then `git fetch <bundle> <ref>:refs/firsthand/snapshots/<id>`.

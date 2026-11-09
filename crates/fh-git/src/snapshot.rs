//! Dirty-state snapshots (master spec 8.1). A snapshot is a commit whose tree is
//! the full working state, including uncommitted and untracked non-ignored
//! files, kept alive by a private ref. It never touches the real index, HEAD,
//! branches or working files.

use crate::repo::Repo;
use crate::runner::split_nul;
use fh_core::id::{IdGen, Snp};
use fh_core::policy::SnapshotPolicy;
use fh_core::{
    B3, Clock, DirtySummary, Error, GitOid, Id, OmitReason, Omitted, Result, SecretScanner,
    SnapshotKind, SnapshotRef,
};
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Bytes of each candidate file passed to the secret scanner.
pub const SCAN_BYTES: usize = 64 * 1024;

/// Result of building a tree from the working state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeOnly {
    /// Tree oid.
    pub tree: GitOid,
    /// Counts and omitted files.
    pub dirty: DirtySummary,
}

/// Temp index that is deleted on drop.
struct TempIndex(PathBuf);

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(self.0.with_extension("lock"));
    }
}

fn build_globs(policy: &SnapshotPolicy) -> Result<GlobSet> {
    let mut b = GlobSetBuilder::new();
    for g in &policy.exclude_globs {
        let glob = Glob::new(g)
            .map_err(|e| Error::Policy(format!("bad snapshot.exclude_globs entry {g:?}: {e}")))?;
        b.add(glob);
        // Also match at the repo root for patterns like **/.env.
        if let Some(rest) = g.strip_prefix("**/")
            && let Ok(g2) = Glob::new(rest)
        {
            b.add(g2);
        }
    }
    b.build()
        .map_err(|e| Error::Policy(format!("snapshot.exclude_globs: {e}")))
}

fn hash_file(p: &Path) -> Option<B3> {
    let mut f = std::fs::File::open(p).ok()?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                h.update(&buf[..n]);
            }
            Err(_) => return None,
        }
    }
    B3::parse(&format!("b3:{}", h.finalize().to_hex())).ok()
}

fn looks_text(b: &[u8]) -> bool {
    !b.contains(&0)
}

impl Repo {
    /// Paths that a snapshot would capture beyond HEAD: untracked non-ignored
    /// files plus files that differ from HEAD (or everything in the index when unborn).
    pub fn snapshot_candidates(&self) -> Result<Vec<String>> {
        let mut v =
            split_nul(
                &self
                    .git
                    .run(&["ls-files", "-z", "--others", "--exclude-standard"])?,
            );
        let changed = if self.head()?.is_some() {
            self.git
                .run(&["diff", "--name-only", "-z", "--no-renames", "HEAD", "--"])?
        } else {
            self.git.run(&["ls-files", "-z", "--cached"])?
        };
        v.extend(split_nul(&changed));
        v.sort();
        v.dedup();
        Ok(v)
    }

    /// Decide which candidates to omit and why.
    fn omissions(
        &self,
        policy: &SnapshotPolicy,
        scanner: &dyn SecretScanner,
    ) -> Result<Vec<Omitted>> {
        let globs = build_globs(policy)?;
        let mut out = Vec::new();
        for path in self.snapshot_candidates()? {
            let full = self.root.join(&path);
            let Ok(meta) = std::fs::symlink_metadata(&full) else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            if globs.is_match(&path) {
                out.push(Omitted {
                    path,
                    reason: OmitReason::SecretPattern,
                    bytes: Some(meta.len()),
                    hash: None,
                });
                continue;
            }
            if meta.len() > policy.max_file_bytes {
                out.push(Omitted {
                    hash: hash_file(&full),
                    path,
                    reason: OmitReason::TooLarge,
                    bytes: Some(meta.len()),
                });
                continue;
            }
            let mut head = Vec::with_capacity(SCAN_BYTES.min(meta.len() as usize));
            if let Ok(f) = std::fs::File::open(&full) {
                let _ = f.take(SCAN_BYTES as u64).read_to_end(&mut head);
            }
            if looks_text(&head) && scanner.scan(&head).is_some() {
                out.push(Omitted {
                    path,
                    reason: OmitReason::SecretContent,
                    bytes: Some(meta.len()),
                    hash: None,
                });
            }
        }
        Ok(out)
    }

    fn temp_index(&self, tag: &str) -> Result<TempIndex> {
        let dir = self.store().tmp_dir();
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        let tmp = dir.join(format!("index-{tag}-{}", std::process::id()));
        let real = self.index_path()?;
        if real.exists() {
            std::fs::copy(&real, &tmp).map_err(|e| Error::io(&real, e))?;
        } else {
            let _ = std::fs::remove_file(&tmp);
        }
        Ok(TempIndex(tmp))
    }

    /// Steps 1 to 3 of the snapshot algorithm: tree oid of the full working state,
    /// without a commit or ref.
    pub fn tree_only(
        &self,
        policy: &SnapshotPolicy,
        scanner: &dyn SecretScanner,
    ) -> Result<TreeOnly> {
        let omitted = self.omissions(policy, scanner)?;
        let idx = self.temp_index("tree")?;
        let env: [(&str, &OsStr); 1] = [("GIT_INDEX_FILE", idx.0.as_os_str())];
        let mut spec = b".\0".to_vec();
        for o in &omitted {
            spec.extend_from_slice(b":(exclude,literal)");
            spec.extend_from_slice(o.path.as_bytes());
            spec.push(0);
        }
        self.git.run_with(
            &[
                "add",
                "-A",
                "--ignore-errors",
                "--pathspec-from-file=-",
                "--pathspec-file-nul",
            ],
            &env,
            Some(&spec),
        )?;
        let tree = GitOid::parse(
            String::from_utf8_lossy(&self.git.run_with(&["write-tree"], &env, None)?).trim(),
        )?;
        let mut dirty = self.dirty_summary()?;
        dirty.omitted = omitted;
        Ok(TreeOnly { tree, dirty })
    }

    /// Full snapshot: tree, commit with a fixed `firsthand` identity (set only in
    /// the environment of that one call), and a ref. Prints nothing; callers warn
    /// about `dirty.omitted`.
    pub fn snapshot_dirty(
        &self,
        kind: SnapshotKind,
        policy: &SnapshotPolicy,
        scanner: &dyn SecretScanner,
        clock: &dyn Clock,
        ids: &IdGen,
    ) -> Result<(SnapshotRef, DirtySummary)> {
        let t = self.tree_only(policy, scanner)?;
        let now = clock.now();
        let id: Id<Snp> = ids.new_id_salted(&now, t.tree.as_str().as_bytes());
        let base = self.head()?;
        let date = format!("@{} +0000", now.unix_ms().div_euclid(1000));
        let env: [(&str, &OsStr); 6] = [
            ("GIT_AUTHOR_NAME", OsStr::new("firsthand")),
            ("GIT_AUTHOR_EMAIL", OsStr::new("firsthand@localhost")),
            ("GIT_COMMITTER_NAME", OsStr::new("firsthand")),
            ("GIT_COMMITTER_EMAIL", OsStr::new("firsthand@localhost")),
            ("GIT_AUTHOR_DATE", OsStr::new(&date)),
            ("GIT_COMMITTER_DATE", OsStr::new(&date)),
        ];
        let msg = format!("firsthand snapshot {id}");
        let mut args: Vec<&str> =
            vec!["-c", "commit.gpgSign=false", "commit-tree", t.tree.as_str()];
        if let Some(b) = &base {
            args.extend(["-p", b.as_str()]);
        }
        args.extend(["-m", &msg]);
        let commit =
            GitOid::parse(String::from_utf8_lossy(&self.git.run_with(&args, &env, None)?).trim())?;
        let ref_name = match kind {
            SnapshotKind::Auto => self.next_auto_ref()?,
            _ => format!("refs/firsthand/snapshots/{id}"),
        };
        self.git.run(&["update-ref", &ref_name, commit.as_str()])?;
        Ok((
            SnapshotRef {
                id,
                commit,
                tree: t.tree,
                base,
                ref_name,
                kind,
                created_at: now,
            },
            t.dirty,
        ))
    }

    /// Next ref name in the rotating `refs/firsthand/auto/<n>` series.
    fn next_auto_ref(&self) -> Result<String> {
        let out = self.git.run(&[
            "for-each-ref",
            "--format=%(refname)",
            "refs/firsthand/auto/",
        ])?;
        let max = String::from_utf8_lossy(&out)
            .lines()
            .filter_map(|l| l.rsplit('/').next()?.parse::<u64>().ok())
            .max();
        Ok(format!("refs/firsthand/auto/{}", max.map_or(0, |m| m + 1)))
    }

    /// List snapshot refs with their commits, newest ref names last.
    pub fn list_snapshot_refs(&self) -> Result<Vec<(String, GitOid)>> {
        let out = self.git.run(&[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/firsthand/snapshots/",
            "refs/firsthand/auto/",
        ])?;
        String::from_utf8_lossy(&out)
            .lines()
            .filter_map(|l| l.split_once(' '))
            .map(|(r, o)| Ok((r.to_owned(), GitOid::parse(o)?)))
            .collect()
    }
}

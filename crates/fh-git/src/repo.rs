//! Repository discovery and read-only queries.

use crate::runner::{GitRunner, split_nul};
use fh_core::store::StoreLayout;
use fh_core::{B3, DirtySummary, Error, GitOid, ObjectFormat, Result, WorktreeInfo};
use std::path::{Path, PathBuf};

/// A discovered repository and worktree.
#[derive(Debug, Clone)]
pub struct Repo {
    /// Top of the working tree.
    pub root: PathBuf,
    /// This worktree's git dir.
    pub git_dir: PathBuf,
    /// The common dir shared by all worktrees.
    pub common_dir: PathBuf,
    /// Object format.
    pub object_format: ObjectFormat,
    /// Runner rooted at `root`.
    pub git: GitRunner,
}

/// One entry of `git ls-tree -r`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    /// File mode, e.g. `100644`, `120000`, `160000`.
    pub mode: String,
    /// `blob`, `tree` or `commit`.
    pub kind: String,
    /// Object id.
    pub oid: GitOid,
    /// Path with `/` separators.
    pub path: String,
}

fn abs(base: &Path, p: &str) -> PathBuf {
    let pb = PathBuf::from(p);
    if pb.is_absolute() { pb } else { base.join(pb) }
}

impl Repo {
    /// Discover the repository containing `start`.
    pub fn discover(start: &Path) -> Result<Repo> {
        let probe = GitRunner::new(start);
        probe.version()?;
        let out = probe
            .run_opt(&["rev-parse", "--show-toplevel", "--git-dir", "--git-common-dir"])?
            .ok_or_else(|| {
                Error::Environment(format!(
                    "not a git repository (or no working tree): {} (run firsthand inside a git checkout)",
                    start.display()
                ))
            })?;
        let mut lines = out.lines();
        let (Some(top), Some(gd), Some(cd)) = (lines.next(), lines.next(), lines.next()) else {
            return Err(Error::Environment(format!(
                "unexpected rev-parse output: {out:?}"
            )));
        };
        let root = PathBuf::from(top);
        let git_dir = abs(start, gd);
        let common_dir = abs(start, cd);
        let fmt = probe
            .run_opt(&["rev-parse", "--show-object-format"])?
            .unwrap_or_default();
        let object_format = if fmt.trim() == "sha256" {
            ObjectFormat::Sha256
        } else {
            ObjectFormat::Sha1
        };
        Ok(Repo {
            git: GitRunner::new(&root),
            root,
            git_dir: dunce(git_dir),
            common_dir: dunce(common_dir),
            object_format,
        })
    }

    /// Store layout under the common dir.
    pub fn store(&self) -> StoreLayout {
        StoreLayout::for_common_dir(&self.common_dir)
    }

    /// `git --version`.
    pub fn git_version(&self) -> Result<String> {
        self.git.version()
    }

    /// HEAD commit, `None` when unborn.
    pub fn head(&self) -> Result<Option<GitOid>> {
        match self
            .git
            .run_opt(&["rev-parse", "--verify", "-q", "HEAD^{commit}"])?
        {
            Some(s) => Ok(Some(GitOid::parse(s.trim())?)),
            None => Ok(None),
        }
    }

    /// Current branch, `None` when detached.
    pub fn branch(&self) -> Result<Option<String>> {
        self.git.run_opt(&["symbolic-ref", "-q", "--short", "HEAD"])
    }

    /// Upstream of the current branch.
    pub fn upstream(&self) -> Result<Option<String>> {
        let out = self.git.output_with(
            &[
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                "@{upstream}",
            ],
            &[],
            None,
        )?;
        Ok(out
            .status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
            .filter(|s| !s.is_empty()))
    }

    /// Stable repo id: BLAKE3 of the newline-joined sorted root commits reachable from HEAD.
    /// Unborn repositories hash the empty string until the first commit.
    pub fn repo_id(&self) -> Result<B3> {
        let roots = match self.head()? {
            Some(_) => self.git.run_str(&["rev-list", "--max-parents=0", "HEAD"])?,
            None => String::new(),
        };
        let mut v: Vec<&str> = roots.lines().filter(|l| !l.is_empty()).collect();
        v.sort_unstable();
        Ok(B3::of(v.join("\n").as_bytes()))
    }

    /// Linked-worktree information.
    pub fn worktree_info(&self) -> WorktreeInfo {
        let linked = self.git_dir != self.common_dir;
        WorktreeInfo {
            linked,
            name: linked
                .then(|| {
                    self.git_dir
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                })
                .flatten(),
        }
    }

    /// Path of this worktree's real index file.
    pub fn index_path(&self) -> Result<PathBuf> {
        let p = self.git.run_str(&["rev-parse", "--git-path", "index"])?;
        Ok(abs(&self.root, &p))
    }

    /// Read a blob's bytes.
    pub fn read_blob(&self, oid: &GitOid) -> Result<Vec<u8>> {
        self.git.run(&["cat-file", "blob", oid.as_str()])
    }

    /// Recursive listing of a tree (or commit).
    pub fn ls_tree(&self, treeish: &str) -> Result<Vec<TreeEntry>> {
        let out = self
            .git
            .run(&["ls-tree", "-r", "-z", "--full-tree", treeish])?;
        parse_ls_tree(&out)
    }

    /// Blob oid of `path` in `treeish`, or `None` if absent.
    pub fn blob_at(&self, treeish: &str, path: &str) -> Result<Option<GitOid>> {
        let out = self
            .git
            .run(&["ls-tree", "-z", "--full-tree", treeish, "--", path])?;
        Ok(parse_ls_tree(&out)?
            .into_iter()
            .find(|e| e.path == path && e.kind == "blob")
            .map(|e| e.oid))
    }

    /// Whether an object exists locally.
    pub fn has_object(&self, oid: &GitOid) -> Result<bool> {
        let out = self
            .git
            .output_with(&["cat-file", "-e", oid.as_str()], &[], None)?;
        Ok(out.status.success())
    }

    /// Counts of uncommitted changes from `git status --porcelain=v2 -z`. Never writes the index.
    pub fn dirty_summary(&self) -> Result<DirtySummary> {
        let out = self
            .git
            .run(&["status", "--porcelain=v2", "-z", "--untracked-files=all"])?;
        Ok(parse_status_v2(&out))
    }
}

/// Parse `git ls-tree -z` output.
pub fn parse_ls_tree(out: &[u8]) -> Result<Vec<TreeEntry>> {
    let mut v = Vec::new();
    for rec in split_nul(out) {
        let (meta, path) = rec
            .split_once('\t')
            .ok_or_else(|| Error::Other(format!("bad ls-tree record {rec:?}")))?;
        let mut m = meta.split(' ');
        let (Some(mode), Some(kind), Some(oid)) = (m.next(), m.next(), m.next()) else {
            return Err(Error::Other(format!("bad ls-tree record {rec:?}")));
        };
        v.push(TreeEntry {
            mode: mode.to_owned(),
            kind: kind.to_owned(),
            oid: GitOid::parse(oid)?,
            path: path.to_owned(),
        });
    }
    Ok(v)
}

/// Parse porcelain v2 `-z` status into counts.
pub fn parse_status_v2(out: &[u8]) -> DirtySummary {
    let mut d = DirtySummary::default();
    let mut recs = out.split(|c| *c == 0).filter(|s| !s.is_empty());
    while let Some(r) = recs.next() {
        match r.first() {
            Some(b'1') | Some(b'2') => {
                let xy = r.get(2..4).unwrap_or(b"..");
                if xy.contains(&b'D') {
                    d.deleted += 1;
                } else if xy.contains(&b'A') {
                    d.added += 1;
                } else {
                    d.modified += 1;
                }
                if r[0] == b'2' {
                    recs.next(); // original path of a rename or copy
                }
            }
            Some(b'u') => d.modified += 1,
            Some(b'?') => d.untracked += 1,
            _ => {}
        }
    }
    d
}

/// Strip the `\\?\` prefix Windows canonicalization can add.
fn dunce(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => p,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_parse() {
        let s =
            b"1 .M N... 100644 100644 100644 a b f1\x001 D. N... 100644 000000 000000 a b f2\x00\
2 R. N... 100644 100644 100644 a b R100 new\x00old\x00? u1\x00? u 2\x00";
        let d = parse_status_v2(s);
        assert_eq!((d.modified, d.deleted, d.untracked, d.added), (2, 1, 2, 0));
    }
}

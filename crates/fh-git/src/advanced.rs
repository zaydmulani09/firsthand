//! History relations (implemented in Phase 0 so drift can be built in parallel)
//! and the frozen signatures Track A implements: bundles, worktrees, restore,
//! auto snapshot rotation. Until Track A merges, those return
//! `Error::NotYetImplemented("A")`.

use crate::repo::Repo;
use crate::runner::split_nul;
use fh_core::{Error, GitOid, HeadRelation, PathChange, Result, SnapshotRef};
use std::path::Path;

/// How `restore --apply` treats files absent from the target snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreMode {
    /// Overwrite files present in the snapshot; leave others.
    Overlay,
    /// Also remove tracked or previously snapshotted files absent from the target.
    Exact,
}

/// Parse `git diff --name-status -z` output.
pub fn parse_name_status(out: &[u8]) -> Vec<PathChange> {
    let items = split_nul(out);
    let mut v = Vec::new();
    let mut it = items.into_iter();
    while let Some(st) = it.next() {
        let status: String = st.chars().take(1).collect();
        if status == "R" || status == "C" {
            let (Some(old), Some(new)) = (it.next(), it.next()) else {
                break;
            };
            v.push(PathChange {
                status,
                path: new,
                old_path: Some(old),
            });
        } else if let Some(p) = it.next() {
            v.push(PathChange {
                status,
                path: p,
                old_path: None,
            });
        }
    }
    v
}

impl Repo {
    /// Relation of HEAD to `anchor`, from `git rev-list --left-right --count`.
    pub fn head_relation(&self, anchor: &GitOid) -> Result<HeadRelation> {
        if !self.has_object(anchor)? {
            return Ok(HeadRelation::AnchorMissing);
        }
        let Some(head) = self.head()? else {
            return Ok(HeadRelation::Unrelated);
        };
        if &head == anchor {
            return Ok(HeadRelation::Same);
        }
        let mb =
            self.git
                .output_with(&["merge-base", head.as_str(), anchor.as_str()], &[], None)?;
        if !mb.status.success() {
            return Ok(HeadRelation::Unrelated);
        }
        let range = format!("{}...{}", head.as_str(), anchor.as_str());
        let s = self
            .git
            .run_str(&["rev-list", "--left-right", "--count", &range])?;
        let mut parts = s.split_whitespace().map(|x| x.parse::<u32>().unwrap_or(0));
        let (ahead, behind) = (parts.next().unwrap_or(0), parts.next().unwrap_or(0));
        Ok(match (ahead, behind) {
            (0, 0) => HeadRelation::Same,
            (n, 0) => HeadRelation::Ahead { n },
            (0, n) => HeadRelation::Behind { n },
            (ahead, behind) => HeadRelation::Diverged { ahead, behind },
        })
    }

    /// Changes between two tree-ish values, with rename detection.
    pub fn diff_name_status(&self, a: &str, b: &str) -> Result<Vec<PathChange>> {
        let out = self
            .git
            .run(&["diff", "--name-status", "-z", "-M", a, b, "--"])?;
        Ok(parse_name_status(&out))
    }

    /// Create a thin bundle of `snapshot_commit` excluding `base`.
    pub fn bundle_create(
        &self,
        _snapshot_commit: &GitOid,
        _base: Option<&GitOid>,
        _out: &Path,
    ) -> Result<()> {
        Err(Error::NotYetImplemented("A"))
    }

    /// Verify a bundle and fetch it into `refs/firsthand/snapshots/<id>`.
    pub fn bundle_materialize(&self, _bundle: &Path, _snapshot: &SnapshotRef) -> Result<()> {
        Err(Error::NotYetImplemented("A"))
    }

    /// `git worktree add --detach <dir> <commit>`.
    pub fn worktree_add_detached(&self, _dir: &Path, _commit: &GitOid) -> Result<()> {
        Err(Error::NotYetImplemented("A"))
    }

    /// `git worktree remove --force <dir>`.
    pub fn worktree_remove(&self, _dir: &Path) -> Result<()> {
        Err(Error::NotYetImplemented("A"))
    }

    /// Restore a snapshot into its own detached worktree at `dir`.
    pub fn restore_to(&self, _snapshot: &GitOid, _dir: &Path) -> Result<()> {
        Err(Error::NotYetImplemented("A"))
    }

    /// Restore a snapshot onto the current worktree after taking a `PreRestore`
    /// snapshot, which is returned.
    pub fn restore_apply(&self, _snapshot: &GitOid, _mode: RestoreMode) -> Result<SnapshotRef> {
        Err(Error::NotYetImplemented("A"))
    }

    /// Delete auto snapshot refs beyond the newest `keep`. Returns how many were deleted.
    pub fn rotate_auto(&self, _keep: u32) -> Result<u32> {
        Err(Error::NotYetImplemented("A"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_status() {
        let v = parse_name_status(b"M\0a.rs\0R087\0old.rs\0new.rs\0D\0gone b.rs\0");
        assert_eq!(v.len(), 3);
        assert_eq!(v[1].old_path.as_deref(), Some("old.rs"));
        assert_eq!(v[1].path, "new.rs");
        assert_eq!(v[2].path, "gone b.rs");
    }
}

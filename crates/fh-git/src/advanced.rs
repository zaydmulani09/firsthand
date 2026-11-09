//! Frozen signatures for the git operations Track A implements: bundles,
//! worktrees, restore, history relations. Until Track A merges, these return
//! `Error::NotYetImplemented("A")`.

use crate::repo::Repo;
use fh_core::{Error, GitOid, Result, SnapshotRef};
use std::path::Path;

/// Relation of the current HEAD to an anchor commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadRelation {
    /// Same commit.
    Same,
    /// HEAD is `n` commits ahead of the anchor.
    Ahead(u32),
    /// HEAD is `n` commits behind the anchor.
    Behind(u32),
    /// Both sides have commits.
    Diverged {
        /// Commits only in HEAD.
        ahead: u32,
        /// Commits only in the anchor.
        behind: u32,
    },
    /// No common history.
    Unrelated,
    /// The anchor commit is not in this clone.
    AnchorMissing,
}

/// One entry of `git diff --name-status -z`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathChange {
    /// `A`, `M`, `D`, `R`, `C`, `T`.
    pub status: char,
    /// Path (new path for renames).
    pub path: String,
    /// Old path for renames and copies.
    pub old_path: Option<String>,
}

/// How `restore --apply` treats files absent from the target snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreMode {
    /// Overwrite files present in the snapshot; leave others.
    Overlay,
    /// Also remove tracked or previously snapshotted files absent from the target.
    Exact,
}

impl Repo {
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

    /// Relation of HEAD to `anchor`.
    pub fn head_relation(&self, _anchor: &GitOid) -> Result<HeadRelation> {
        Err(Error::NotYetImplemented("A"))
    }

    /// Changes between two tree-ish values.
    pub fn diff_name_status(&self, _a: &str, _b: &str) -> Result<Vec<PathChange>> {
        Err(Error::NotYetImplemented("A"))
    }

    /// Restore a snapshot into its own detached worktree at `dir`.
    pub fn restore_to(&self, _snapshot: &GitOid, _dir: &Path) -> Result<()> {
        Err(Error::NotYetImplemented("A"))
    }

    /// Restore a snapshot onto the current worktree after taking a `PreRestore` snapshot.
    pub fn restore_apply(&self, _snapshot: &GitOid, _mode: RestoreMode) -> Result<SnapshotRef> {
        Err(Error::NotYetImplemented("A"))
    }

    /// Delete auto snapshot refs beyond the newest `keep`.
    pub fn rotate_auto(&self, _keep: u32) -> Result<u32> {
        Err(Error::NotYetImplemented("A"))
    }
}

//! On-disk layout of the store (`<git-common-dir>/firsthand/`) and the user config dir.

use std::path::{Path, PathBuf};

/// Paths inside the store root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreLayout {
    /// `<git-common-dir>/firsthand`.
    pub root: PathBuf,
}

impl StoreLayout {
    /// Layout for a git common dir.
    pub fn for_common_dir(common_dir: &Path) -> Self {
        StoreLayout {
            root: common_dir.join("firsthand"),
        }
    }
    /// `ledger/` directory.
    pub fn ledger_dir(&self) -> PathBuf {
        self.root.join("ledger")
    }
    /// Ledger file for a leg.
    pub fn ledger_file(&self, leg_id: &str) -> PathBuf {
        self.ledger_dir().join(format!("{leg_id}.jsonl"))
    }
    /// `blobs/` directory.
    pub fn blobs_dir(&self) -> PathBuf {
        self.root.join("blobs")
    }
    /// Path of a blob by hex hash: `blobs/<aa>/<hex>`.
    pub fn blob_path(&self, hex: &str) -> PathBuf {
        self.blobs_dir().join(&hex[..2]).join(hex)
    }
    /// `packets/` directory.
    pub fn packets_dir(&self) -> PathBuf {
        self.root.join("packets")
    }
    /// `index.db`.
    pub fn index_db(&self) -> PathBuf {
        self.root.join("index.db")
    }
    /// Repo-local `policy.toml`.
    pub fn policy(&self) -> PathBuf {
        self.root.join("policy.toml")
    }
    /// `pending.json` (armed handoff).
    pub fn pending(&self) -> PathBuf {
        self.root.join("pending.json")
    }
    /// `pending/` directory for Pre/Post hook pairing.
    pub fn pending_dir(&self) -> PathBuf {
        self.root.join("pending")
    }
    /// Advisory lock file for ledger appends.
    pub fn lock(&self) -> PathBuf {
        self.root.join("LOCK")
    }
    /// Hook error log.
    pub fn hook_errors(&self) -> PathBuf {
        self.root.join("hook-errors.log")
    }
    /// Pointer to the currently open leg.
    pub fn current_leg(&self) -> PathBuf {
        self.root.join("current-leg")
    }
    /// Scratch directory for temp indexes and bundles.
    pub fn tmp_dir(&self) -> PathBuf {
        self.root.join("tmp")
    }
}

/// User config dir: `$FIRSTHAND_HOME`, else the platform config dir for `firsthand`.
pub fn user_config_dir() -> Option<PathBuf> {
    if let Some(h) = std::env::var_os("FIRSTHAND_HOME") {
        return Some(PathBuf::from(h));
    }
    directories::ProjectDirs::from("", "", "firsthand").map(|d| d.config_dir().to_path_buf())
}

/// Files inside the user config dir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserLayout {
    /// The config dir.
    pub root: PathBuf,
}

impl UserLayout {
    /// Layout from [`user_config_dir`].
    pub fn discover() -> Option<Self> {
        user_config_dir().map(|root| UserLayout { root })
    }
    /// Private signing key.
    pub fn private_key(&self) -> PathBuf {
        self.root.join("keys").join("ed25519.key")
    }
    /// Public key.
    pub fn public_key(&self) -> PathBuf {
        self.root.join("keys").join("ed25519.pub")
    }
    /// Trusted keys file.
    pub fn trusted_keys(&self) -> PathBuf {
        self.root.join("trusted_keys.toml")
    }
    /// User-level policy defaults.
    pub fn policy(&self) -> PathBuf {
        self.root.join("policy.toml")
    }
}

/// Working-tree directory for derived briefs.
pub fn briefs_dir(worktree_root: &Path) -> PathBuf {
    worktree_root.join(".firsthand").join("briefs")
}

//! Test helpers: real temporary git repositories, a frozen clock, seeded ids,
//! a minimal secret scanner, and "real repo state unchanged" assertions.
//!
//! Test repositories commit with a fixed identity passed through the
//! environment of each git call; no git config is written anywhere.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use fh_core::id::IdGen;
use fh_core::time::FixedClock;
use fh_core::{SecretScanner, Ts};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Fixed test instant.
pub const TEST_TIME: &str = "2026-11-09T14:02:11.123Z";

/// Clock frozen at [`TEST_TIME`].
pub fn clock() -> FixedClock {
    FixedClock(Ts::parse(TEST_TIME).unwrap())
}

/// Deterministic id generator.
pub fn ids() -> IdGen {
    IdGen::seeded("testkit")
}

/// Minimal scanner for tests: PEM private keys and AWS-style access key ids.
/// The real rules live in `fh-redact`.
#[derive(Debug, Default, Clone, Copy)]
pub struct TestScanner;

impl SecretScanner for TestScanner {
    fn scan(&self, bytes: &[u8]) -> Option<String> {
        let s = String::from_utf8_lossy(bytes);
        if s.contains("-----BEGIN") && s.contains("PRIVATE KEY-----") {
            return Some("pem_private_key".into());
        }
        let b = s.as_bytes();
        for i in 0..b.len().saturating_sub(19) {
            if &b[i..i + 4] == b"AKIA"
                && b[i + 4..i + 20]
                    .iter()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            {
                return Some("aws_access_key_id".into());
            }
        }
        None
    }
}

/// A real git repository in a temp directory.
pub struct TestRepo {
    dir: tempfile::TempDir,
}

impl Default for TestRepo {
    fn default() -> Self {
        Self::new()
    }
}

impl TestRepo {
    /// `git init -b main` in a fresh temp dir.
    pub fn new() -> Self {
        Self::with_args(&[])
    }

    /// `git init --object-format=sha256`, or `None` if this git cannot.
    pub fn sha256() -> Option<Self> {
        let dir = tempfile::tempdir().unwrap();
        let ok = git_cmd(dir.path())
            .args(["init", "-q", "-b", "main", "--object-format=sha256"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        ok.then_some(TestRepo { dir })
    }

    fn with_args(extra: &[&str]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut c = git_cmd(dir.path());
        c.args(["init", "-q", "-b", "main"]).args(extra);
        assert!(c.status().unwrap().success(), "git init failed");
        TestRepo { dir }
    }

    /// Root path.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Run git and return stdout; panics on failure.
    pub fn git(&self, args: &[&str]) -> String {
        run_git(self.path(), args)
    }

    /// Write a file (creating parent dirs).
    pub fn write(&self, rel: &str, content: impl AsRef<[u8]>) -> &Self {
        let p = self.path().join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
        self
    }

    /// Read a file.
    pub fn read(&self, rel: &str) -> Vec<u8> {
        std::fs::read(self.path().join(rel)).unwrap()
    }

    /// Delete a file.
    pub fn remove(&self, rel: &str) -> &Self {
        std::fs::remove_file(self.path().join(rel)).unwrap();
        self
    }

    /// Stage everything and commit; returns the new HEAD.
    pub fn commit_all(&self, msg: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "--allow-empty", "-m", msg]);
        self.head()
    }

    /// HEAD commit.
    pub fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"]).trim().to_owned()
    }

    /// Create `n` commits each touching `file`.
    pub fn linear_history(&self, n: usize, file: &str) -> Vec<String> {
        (0..n)
            .map(|i| {
                self.write(file, format!("line {i}\n"));
                self.commit_all(&format!("commit {i}"))
            })
            .collect()
    }

    /// Add a linked worktree at `dir` on a new branch.
    pub fn add_worktree(&self, dir: &Path, branch: &str) {
        let d = dir.to_string_lossy().into_owned();
        self.git(&["worktree", "add", "-q", "-b", branch, &d]);
    }

    /// File names that stress path handling (spaces, unicode, leading dash).
    /// Newlines are only valid on Unix file systems and are added there.
    pub fn weird_names() -> Vec<&'static str> {
        let mut v = vec![
            "with space.txt",
            "\u{e9}t\u{e9}.txt",
            "-leading-dash.txt",
            "deep/nested dir/f.txt",
            "\u{65e5}\u{672c}.md",
        ];
        if cfg!(unix) {
            v.push("new\nline.txt");
        }
        v
    }

    /// Fingerprint of everything a snapshot must not change.
    pub fn state(&self) -> RepoState {
        RepoState::capture(self.path())
    }
}

/// Real repo state: status bytes, index file hash, HEAD, refs/heads, and every working file hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoState {
    pub status: Vec<u8>,
    pub index_hash: Option<String>,
    pub head: Option<String>,
    pub heads: String,
    pub files: BTreeMap<PathBuf, String>,
}

impl RepoState {
    /// Capture the state of the worktree at `root`.
    pub fn capture(root: &Path) -> Self {
        let status = git_cmd(root)
            .args(["status", "--porcelain=v2", "-z", "--untracked-files=all"])
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
            .unwrap()
            .stdout;
        let gitdir = run_git(root, &["rev-parse", "--git-path", "index"]);
        let idx = root.join(gitdir.trim());
        let index_hash = std::fs::read(&idx)
            .ok()
            .map(|b| blake3::hash(&b).to_hex().to_string());
        let head = git_cmd(root)
            .args(["rev-parse", "-q", "--verify", "HEAD"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());
        let heads = run_git(root, &["for-each-ref", "refs/heads", "refs/tags"]);
        let mut files = BTreeMap::new();
        for e in walkdir::WalkDir::new(root)
            .into_iter()
            .filter_entry(|e| e.file_name() != ".git")
        {
            let e = e.unwrap();
            if e.file_type().is_file() {
                let b = std::fs::read(e.path()).unwrap();
                files.insert(
                    e.path().strip_prefix(root).unwrap().to_path_buf(),
                    blake3::hash(&b).to_hex().to_string(),
                );
            }
        }
        RepoState {
            status,
            index_hash,
            head,
            heads,
            files,
        }
    }
}

fn git_cmd(cwd: &Path) -> Command {
    let mut c = Command::new("git");
    c.current_dir(cwd)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args([
            "-c",
            "core.autocrlf=false",
            "-c",
            "commit.gpgSign=false",
            "-c",
            "init.defaultBranch=main",
        ]);
    for k in ["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE"] {
        c.env_remove(k);
    }
    c
}

/// Run git in `cwd`, panicking with stderr on failure.
pub fn run_git(cwd: &Path, args: &[&str]) -> String {
    let out = git_cmd(cwd).args(args).output().unwrap();
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Load a fixture file relative to a crate's `tests/fixtures` directory.
pub fn fixture(crate_dir: &str, rel: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(crate_dir)
            .join("tests")
            .join("fixtures")
            .join(rel),
    )
    .unwrap()
}

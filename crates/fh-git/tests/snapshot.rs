#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use fh_core::policy::SnapshotPolicy;
use fh_core::{OmitReason, SnapshotKind};
use fh_git::Repo;
use fh_testkit::{TestRepo, TestScanner, clock, ids};

fn snap(r: &TestRepo, policy: &SnapshotPolicy) -> (fh_core::SnapshotRef, fh_core::DirtySummary) {
    let repo = Repo::discover(r.path()).unwrap();
    repo.snapshot_dirty(SnapshotKind::Manual, policy, &TestScanner, &clock(), &ids())
        .unwrap()
}

fn tree_files(r: &TestRepo, commit: &str) -> Vec<String> {
    let repo = Repo::discover(r.path()).unwrap();
    repo.ls_tree(commit)
        .unwrap()
        .into_iter()
        .map(|e| e.path)
        .collect()
}

#[test]
fn captures_dirty_state_without_touching_anything() {
    let r = TestRepo::new();
    r.write("keep.txt", "a\n")
        .write("mod.txt", "1\n")
        .write("del.txt", "x\n");
    r.write(".gitignore", "build/\n");
    r.commit_all("init");
    r.write("mod.txt", "2\n")
        .remove("del.txt")
        .write("new.txt", "fresh\n");
    r.write("build/out.bin", "ignored");
    r.write(".env", "TOKEN=abc\n");
    r.write(
        "notes/key.txt",
        "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----\n",
    );
    r.write("big.bin", vec![7u8; 2048]);
    r.git(&["add", "new.txt"]); // a staged file: index must stay exactly as it is
    let before = r.state();

    let policy = SnapshotPolicy {
        max_file_bytes: 1024,
        ..SnapshotPolicy::default()
    };
    let (s, dirty) = snap(&r, &policy);

    assert_eq!(
        r.state(),
        before,
        "snapshot changed index, HEAD, refs, status or files"
    );
    let files = tree_files(&r, s.commit.as_str());
    assert!(files.contains(&"new.txt".to_owned()));
    assert!(files.contains(&"mod.txt".to_owned()));
    assert!(!files.contains(&"del.txt".to_owned()));
    for absent in ["build/out.bin", ".env", "notes/key.txt", "big.bin"] {
        assert!(
            !files.contains(&absent.to_owned()),
            "{absent} must not be captured"
        );
    }
    let reasons: Vec<(String, OmitReason)> = dirty
        .omitted
        .iter()
        .map(|o| (o.path.clone(), o.reason))
        .collect();
    assert!(reasons.contains(&(".env".into(), OmitReason::SecretPattern)));
    assert!(reasons.contains(&("notes/key.txt".into(), OmitReason::SecretContent)));
    assert!(reasons.contains(&("big.bin".into(), OmitReason::TooLarge)));
    assert!(
        !reasons.iter().any(|(p, _)| p.starts_with("build/")),
        "ignored files are never listed"
    );
    assert_eq!(dirty.deleted, 1);
    assert_eq!(
        s.base.as_ref().map(|b| b.as_str().to_owned()),
        Some(r.head())
    );
    let mod_blob = Repo::discover(r.path())
        .unwrap()
        .blob_at(s.commit.as_str(), "mod.txt")
        .unwrap()
        .unwrap();
    assert_eq!(r.git(&["cat-file", "blob", mod_blob.as_str()]), "2\n");
    assert!(
        r.git(&["for-each-ref", "refs/firsthand/snapshots/"])
            .contains(s.commit.as_str())
    );
    // No branch history: main still points at the original commit.
    assert_eq!(r.git(&["log", "--format=%s", "main"]).trim(), "init");
}

#[test]
fn unborn_repo_and_weird_names() {
    let r = TestRepo::new();
    for n in TestRepo::weird_names() {
        r.write(n, format!("content of {n}"));
    }
    let before = r.state();
    let (s, dirty) = snap(&r, &SnapshotPolicy::default());
    assert_eq!(r.state(), before);
    assert!(s.base.is_none());
    let files = tree_files(&r, s.commit.as_str());
    for n in TestRepo::weird_names() {
        assert!(files.contains(&n.to_owned()), "missing {n:?} in {files:?}");
    }
    assert_eq!(dirty.untracked as usize, TestRepo::weird_names().len());
}

#[test]
fn survives_reset_and_clean() {
    let r = TestRepo::new();
    r.write("a.txt", "v1\n");
    r.commit_all("init");
    r.write("a.txt", "precious edit\n")
        .write("b.txt", "untracked work\n");
    let (s, _) = snap(&r, &SnapshotPolicy::default());
    r.git(&["reset", "-q", "--hard"]);
    r.git(&["clean", "-fdxq"]);
    assert!(!r.path().join("b.txt").exists());
    let repo = Repo::discover(r.path()).unwrap();
    let b = repo.blob_at(s.ref_name.as_str(), "b.txt").unwrap().unwrap();
    assert_eq!(repo.read_blob(&b).unwrap(), b"untracked work\n");
}

#[test]
fn linked_worktree_snapshot_uses_its_own_index() {
    let r = TestRepo::new();
    r.write("a.txt", "main\n");
    r.commit_all("init");
    let wt_parent = tempfile::tempdir().unwrap();
    let wt = wt_parent.path().join("wt");
    r.add_worktree(&wt, "side");
    std::fs::write(wt.join("side.txt"), "side work\n").unwrap();
    let repo = Repo::discover(&wt).unwrap();
    assert!(repo.worktree_info().linked);
    assert_eq!(repo.worktree_info().name.as_deref(), Some("wt"));
    let main_before = r.state();
    let (s, _) = repo
        .snapshot_dirty(
            SnapshotKind::Handoff,
            &SnapshotPolicy::default(),
            &TestScanner,
            &clock(),
            &ids(),
        )
        .unwrap();
    assert_eq!(r.state(), main_before);
    assert!(
        repo.blob_at(s.commit.as_str(), "side.txt")
            .unwrap()
            .is_some()
    );
    assert_eq!(
        repo.common_dir,
        Repo::discover(r.path()).unwrap().common_dir
    );
}

#[test]
fn auto_refs_rotate_numbering() {
    let r = TestRepo::new();
    r.write("a.txt", "1");
    r.commit_all("init");
    let repo = Repo::discover(r.path()).unwrap();
    let p = SnapshotPolicy::default();
    let (a, _) = repo
        .snapshot_dirty(SnapshotKind::Auto, &p, &TestScanner, &clock(), &ids())
        .unwrap();
    r.write("a.txt", "2");
    let (b, _) = repo
        .snapshot_dirty(SnapshotKind::Auto, &p, &TestScanner, &clock(), &ids())
        .unwrap();
    assert_eq!(a.ref_name, "refs/firsthand/auto/0");
    assert_eq!(b.ref_name, "refs/firsthand/auto/1");
    assert_eq!(repo.list_snapshot_refs().unwrap().len(), 2);
}

#[test]
fn identity_and_queries() {
    let r = TestRepo::new();
    let repo = Repo::discover(r.path()).unwrap();
    assert_eq!(repo.head().unwrap(), None);
    r.write("x", "1");
    r.commit_all("init");
    assert_eq!(repo.branch().unwrap().as_deref(), Some("main"));
    let id1 = repo.repo_id().unwrap();
    r.linear_history(3, "x");
    assert_eq!(
        repo.repo_id().unwrap(),
        id1,
        "repo id is stable as history grows"
    );
    assert!(Repo::discover(&std::env::temp_dir().join("definitely-not-a-repo-fh")).is_err());
}

#[test]
fn sha256_repos_work_when_supported() {
    let Some(r) = TestRepo::sha256() else { return };
    r.write("a.txt", "1\n");
    r.commit_all("init");
    r.write("b.txt", "2\n");
    let repo = Repo::discover(r.path()).unwrap();
    assert_eq!(repo.object_format, fh_core::ObjectFormat::Sha256);
    let (s, _) = snap(&r, &SnapshotPolicy::default());
    assert_eq!(s.commit.as_str().len(), 64);
}

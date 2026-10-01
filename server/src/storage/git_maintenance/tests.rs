//! Coverage for consolidating fragmented repositories without losing committed content.

use super::*;
use crate::storage::{
    git_repo::{commit_all_with_progress, prepare_worktree, read_head_blob},
    scratch,
};

/// Commits `count` documents, flushing a separate pack per document.
fn fragmented_repo(dir: &Path, count: usize) -> std::path::PathBuf {
    let bare = dir.join("repo.git");
    let work = dir.join("work");
    let prepared = prepare_worktree(&bare, &work).unwrap();
    for i in 0..count {
        scratch::write(
            &work,
            Path::new(&format!("example.com/advisories/{i}.json")),
            format!("document {i}").as_bytes(),
        )
        .unwrap();
    }
    assert!(commit_all_with_progress(&prepared, "initial", 1, |_, _| {}).unwrap());
    bare
}

/// Many packs collapse into few, temp packs disappear, and HEAD content is preserved.
#[test]
fn consolidates_fragmented_repository() {
    let dir = tempfile::tempdir().unwrap();
    let bare = fragmented_repo(dir.path(), 60);
    let pack_dir = bare.join("objects/pack");
    fs::write(pack_dir.join("pack_git2_abandoned"), b"partial").unwrap();
    assert!(count_packs(&pack_dir).unwrap() > 50);

    maintain_repository(&bare).unwrap();

    assert!(count_packs(&pack_dir).unwrap() <= 2);
    assert!(!pack_dir.join("pack_git2_abandoned").exists());
    for i in 0..60 {
        assert_eq!(
            read_head_blob(&bare, &format!("example.com/advisories/{i}.json"))
                .unwrap()
                .unwrap(),
            format!("document {i}").as_bytes()
        );
    }
}

/// Repositories below Git's thresholds are left as they are.
#[test]
fn leaves_healthy_repository_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let bare = fragmented_repo(dir.path(), 3);
    let pack_dir = bare.join("objects/pack");
    let before = count_packs(&pack_dir).unwrap();

    maintain_repository(&bare).unwrap();

    assert_eq!(count_packs(&pack_dir).unwrap(), before);
}

/// A provider without a repository yet needs no maintenance.
#[test]
fn ignores_missing_repository() {
    let dir = tempfile::tempdir().unwrap();
    maintain_repository(&dir.path().join("missing.git")).unwrap();
}

//! Selects and materializes validation inputs from committed provider snapshots.
use super::{git_repo::url_to_git_path, processing::ProcessingCheckpoint, scratch};
use anyhow::{Context, Result};
use git2::{Oid, Repository, TreeWalkMode, TreeWalkResult};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path};

/// Documents and shared inputs required for a processing pass.
#[derive(Debug)]
pub struct ProcessingSelection {
    /// Snapshot to publish after successful processing.
    pub checkpoint: ProcessingCheckpoint,
    /// Whether every stored advisory must be validated.
    pub full: bool,
    /// Whether history counts require a complete baseline.
    pub baseline: bool,
    /// Advisory paths to validate, including sidecar changes.
    pub advisories: BTreeSet<String>,
    /// Advisory paths whose content history changed.
    pub history: BTreeSet<String>,
    /// Advisory URLs removed from the resulting snapshot.
    pub deleted: Vec<String>,
}

/// Maps an advisory or integrity sidecar to its advisory path.
pub(super) fn advisory_path(path: &str) -> Option<String> {
    if path.starts_with("metadata/") {
        return None;
    }
    let path = [".asc", ".sha256", ".sha512"]
        .iter()
        .find_map(|suffix| path.strip_suffix(suffix))
        .unwrap_or(path);
    path.ends_with(".json").then(|| path.to_string())
}

/// Converts the stored HTTP path layout to a canonical advisory URL.
pub fn path_url(path: &str) -> String {
    format!("https://{path}")
}

/// Selects changes since the last successful processing pass, including reverted changes.
pub fn select_processing(
    repo_path: &Path,
    previous: Option<&ProcessingCheckpoint>,
    validator_identity: &str,
    force_full: bool,
) -> Result<ProcessingSelection> {
    let repo = Repository::open_bare(repo_path)?;
    let head = repo.head()?.peel_to_commit()?;
    let tree = head.tree()?;
    let mut hasher = Sha256::new();
    hasher.update(validator_identity.as_bytes());
    // Include only inputs affecting validation, not provider timestamps or notes.
    if let Ok(metadata) = tree.get_path(Path::new("metadata/provider-metadata.json")) {
        let blob = repo.find_blob(metadata.id())?;
        let value: serde_json::Value = serde_json::from_slice(blob.content())?;
        for key in ["distributions", "public_openpgp_keys"] {
            hasher.update(serde_json::to_vec(&value[key])?);
        }
    }
    if let Ok(keys) = tree.get_path(Path::new("metadata/keys")) {
        hasher.update(keys.id().as_bytes());
    }
    let fingerprint = hex::encode(hasher.finalize());
    let previous_oid = previous.and_then(|p| Oid::from_str(&p.commit_id).ok());
    let mut baseline = previous_oid.is_none();
    let mut changes = BTreeSet::new();
    // First-parent traversal deliberately rejects merges and rewritten history.
    let mut current = head.clone();
    while !baseline && Some(current.id()) != previous_oid {
        if current.parent_count() != 1 {
            baseline = true;
            break;
        }
        let parent = current.parent(0)?;
        let parent_tree = parent.tree()?;
        let current_tree = current.tree()?;
        let diff = repo.diff_tree_to_tree(Some(&parent_tree), Some(&current_tree), None)?;
        for delta in diff.deltas() {
            for file in [delta.old_file(), delta.new_file()] {
                if let Some(path) = file.path().and_then(Path::to_str) {
                    changes.insert(path.to_string());
                }
            }
        }
        current = parent;
    }
    let full = baseline || force_full || previous.is_none_or(|p| p.fingerprint != fingerprint);
    let mut advisories = BTreeSet::new();
    let mut history = BTreeSet::new();
    let mut deleted = Vec::new();
    for path in &changes {
        if let Some(advisory) = advisory_path(path) {
            if path == &advisory {
                history.insert(advisory.clone());
            }
            if tree.get_path(Path::new(&advisory)).is_ok() {
                advisories.insert(advisory);
            } else if path == &advisory {
                deleted.push(path_url(&advisory));
            }
        }
    }
    if full {
        tree.walk(TreeWalkMode::PreOrder, |dir, entry| {
            let path = format!("{dir}{}", entry.name().unwrap_or_default());
            if entry.kind() == Some(git2::ObjectType::Blob)
                && advisory_path(&path).as_deref() == Some(&path)
            {
                advisories.insert(path);
            }
            TreeWalkResult::Ok
        })?;
    }
    let reason = if baseline {
        "missing checkpoint or incompatible history"
    } else if force_full {
        "manual revalidation"
    } else if full {
        "validation inputs changed"
    } else {
        "incremental file changes"
    };
    tracing::info!(
        reason,
        full,
        baseline,
        selected = advisories.len(),
        history = history.len(),
        "Selected provider processing inputs"
    );
    Ok(ProcessingSelection {
        checkpoint: ProcessingCheckpoint {
            commit_id: head.id().to_string(),
            fingerprint,
            summary_dirty: false,
        },
        full,
        baseline,
        advisories,
        history,
        deleted,
    })
}

/// Adds successfully redownloaded advisories with persisted retrieval failures.
pub fn include_recovered_downloads(
    selection: &mut ProcessingSelection,
    urls: &[String],
    download_dir: &Path,
) -> Result<()> {
    for url in urls {
        if let Some(path) = url_to_git_path(url)?
            && (download_dir.join(&path).exists()
                || scratch::compressed_path(&download_dir.join(&path)).exists())
        {
            selection.advisories.insert(path);
        }
    }
    Ok(())
}

/// Materializes selected advisories and shared metadata from one immutable snapshot.
pub fn materialize_processing(
    repo_path: &Path,
    selection: &ProcessingSelection,
    output: &Path,
) -> Result<()> {
    std::fs::create_dir_all(output)?;
    let repo = Repository::open_bare(repo_path)?;
    let tree = repo
        .find_commit(Oid::from_str(&selection.checkpoint.commit_id)?)?
        .tree()?;
    let mut paths = BTreeSet::new();
    if let Ok(entry) = tree.get_path(Path::new("metadata")) {
        let metadata = repo.find_tree(entry.id())?;
        metadata.walk(TreeWalkMode::PreOrder, |dir, entry| {
            if entry.kind() == Some(git2::ObjectType::Blob) {
                paths.insert(format!(
                    "metadata/{dir}{}",
                    entry.name().unwrap_or_default()
                ));
            }
            TreeWalkResult::Ok
        })?;
    }
    for advisory in &selection.advisories {
        paths.insert(advisory.clone());
        for suffix in [".asc", ".sha256", ".sha512"] {
            let path = format!("{advisory}{suffix}");
            if tree.get_path(Path::new(&path)).is_ok() {
                paths.insert(path);
            }
        }
    }
    for path in paths {
        let entry = tree
            .get_path(Path::new(&path))
            .with_context(|| format!("Missing selected input {path}"))?;
        let blob = repo.find_blob(entry.id())?;
        scratch::write(output, Path::new(&path), blob.content())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::git_repo::{collect_versions, commit_snapshot, prepare_worktree};
    use std::fs;

    /// Publishes exact downloaded bytes without checking out unchanged advisories.
    fn commit(repo: &Path, work: &Path, files: &[(&str, &[u8])]) {
        let prepared = prepare_worktree(repo, work).unwrap();
        for (path, data) in files {
            scratch::write(work, Path::new(path), data).unwrap();
        }
        commit_snapshot(&prepared, "test").unwrap();
    }

    /// Unchanged downloads and timestamp-only metadata changes select no advisories.
    #[test]
    fn unchanged_and_metadata_only_skip_documents() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo.git");
        let work = dir.path().join("work");
        commit(
            &repo,
            &work,
            &[
                ("example.com/a.json", b"first"),
                (
                    "metadata/provider-metadata.json",
                    br#"{"last_updated":"old"}"#,
                ),
            ],
        );
        let initial = select_processing(&repo, None, "v1", false).unwrap();
        assert!(initial.baseline);
        assert_eq!(initial.advisories.len(), 1);
        commit(&repo, &work, &[("example.com/a.json", b"first")]);
        let unchanged = select_processing(&repo, Some(&initial.checkpoint), "v1", false).unwrap();
        assert!(!unchanged.full);
        assert!(unchanged.advisories.is_empty());
        assert!(unchanged.history.is_empty());
        commit(
            &repo,
            &work,
            &[(
                "metadata/provider-metadata.json",
                br#"{"last_updated":"new"}"#,
            )],
        );
        let metadata = select_processing(&repo, Some(&initial.checkpoint), "v1", false).unwrap();
        assert!(!metadata.full);
        assert!(metadata.advisories.is_empty());
        assert!(metadata.history.is_empty());
    }

    /// Interrupted work is recovered even if a later commit reverts its bytes.
    #[test]
    fn reverted_changes_remain_pending_and_record_versions() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo.git");
        let work = dir.path().join("work");
        commit(
            &repo,
            &work,
            &[
                ("example.com/a.json", b"first"),
                ("example.com/b.json", b"untouched"),
            ],
        );
        let initial = select_processing(&repo, None, "v1", false).unwrap();
        commit(&repo, &work, &[("example.com/a.json", b"second")]);
        commit(&repo, &work, &[("example.com/a.json", b"first")]);
        let selected = select_processing(&repo, Some(&initial.checkpoint), "v1", false).unwrap();
        assert_eq!(
            selected.advisories,
            BTreeSet::from(["example.com/a.json".into()])
        );
        assert_eq!(selected.history, selected.advisories);
        let mut since_checkpoint = 0;
        collect_versions(&repo, Some(&initial.checkpoint.commit_id), |commit| {
            since_checkpoint += commit.documents.len();
            Ok(())
        })
        .unwrap();
        assert_eq!(since_checkpoint, 2);
        let validation = dir.path().join("validation");
        materialize_processing(&repo, &selected, &validation).unwrap();
        assert_eq!(
            scratch::read(&validation.join("example.com/a.json.zst")).unwrap(),
            b"first"
        );
        assert!(!validation.join("example.com/b.json.zst").exists());
    }

    /// Integrity sidecars trigger validation without changing advisory revision counts.
    #[test]
    fn sidecars_and_key_rotation_invalidate_correct_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo.git");
        let work = dir.path().join("work");
        commit(
            &repo,
            &work,
            &[
                ("example.com/a.json", b"first"),
                ("example.com/a.json.asc", b"signature"),
                ("metadata/keys/0.key", b"key"),
                ("metadata/keys/1.key", b"retired"),
            ],
        );
        let initial = select_processing(&repo, None, "v1", false).unwrap();
        commit(
            &repo,
            &work,
            &[("example.com/a.json.asc", b"new signature")],
        );
        let sidecar = select_processing(&repo, Some(&initial.checkpoint), "v1", false).unwrap();
        assert_eq!(sidecar.advisories.len(), 1);
        assert!(sidecar.history.is_empty());
        commit(&repo, &work, &[("example.com/a.json", b"first")]);
        let removed = select_processing(&repo, Some(&sidecar.checkpoint), "v1", false).unwrap();
        assert_eq!(removed.advisories.len(), 1);
        assert!(removed.history.is_empty());
        commit(&repo, &work, &[("metadata/keys/0.key", b"key")]);
        let rotated = select_processing(&repo, Some(&removed.checkpoint), "v1", false).unwrap();
        assert!(rotated.full);
        assert!(!rotated.baseline);
        assert!(rotated.history.is_empty());
        assert!(
            super::super::git_repo::read_head_blob(&repo, "metadata/keys/1.key")
                .unwrap()
                .is_none()
        );
        assert!(
            select_processing(&repo, Some(&rotated.checkpoint), "v2", false)
                .unwrap()
                .full
        );
        assert!(
            select_processing(&repo, Some(&rotated.checkpoint), "v1", true)
                .unwrap()
                .full
        );
        let invalid = ProcessingCheckpoint {
            commit_id: "missing".into(),
            fingerprint: "v1".into(),
            summary_dirty: false,
        };
        assert!(
            select_processing(&repo, Some(&invalid), "v1", false)
                .unwrap()
                .baseline
        );
    }

    /// Renames are treated as removing the old URL and validating the new one.
    #[test]
    fn deletion_and_rename_are_explicit_git_changes() {
        let dir = tempfile::tempdir().unwrap();
        let repo_path = dir.path().join("repo.git");
        commit(
            &repo_path,
            &dir.path().join("work"),
            &[("example.com/a.json", b"first")],
        );
        let initial = select_processing(&repo_path, None, "v1", false).unwrap();
        let repo = Repository::open_bare(&repo_path).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        let mut index = git2::Index::new().unwrap();
        index.read_tree(&parent.tree().unwrap()).unwrap();
        let mut entry = index.get_path(Path::new("example.com/a.json"), 0).unwrap();
        index.remove_path(Path::new("example.com/a.json")).unwrap();
        entry.path = b"example.com/renamed.json".to_vec();
        index.add(&entry).unwrap();
        let tree = repo.find_tree(index.write_tree_to(&repo).unwrap()).unwrap();
        let signature = git2::Signature::now("test", "test@example.com").unwrap();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "rename",
            &tree,
            &[&parent],
        )
        .unwrap();
        let selected =
            select_processing(&repo_path, Some(&initial.checkpoint), "v1", false).unwrap();
        assert_eq!(selected.deleted, ["https://example.com/a.json"]);
        assert_eq!(
            selected.advisories,
            BTreeSet::from(["example.com/renamed.json".into()])
        );
    }

    /// Measures unchanged selection against a large tree and substantial provider history.
    #[test]
    #[ignore = "manual performance fixture: 10000 documents and 100 commits"]
    fn benchmark_unchanged_selection() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo.git");
        let work = dir.path().join("work");
        let prepared = prepare_worktree(&repo, &work).unwrap();
        fs::create_dir_all(work.join("example.com")).unwrap();
        for n in 0..10_000 {
            fs::write(work.join(format!("example.com/{n}.json")), b"{}").unwrap();
        }
        commit_snapshot(&prepared, "baseline").unwrap();
        for n in 0..100 {
            commit(
                &repo,
                &work,
                &[(
                    "metadata/provider-metadata.json",
                    format!("{{\"last_updated\":{n}}}").as_bytes(),
                )],
            );
        }
        let initial = select_processing(&repo, None, "v1", false).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..100 {
            let selection =
                select_processing(&repo, Some(&initial.checkpoint), "v1", false).unwrap();
            assert!(!selection.full);
            assert!(selection.advisories.is_empty());
            assert!(selection.history.is_empty());
        }
        eprintln!(
            "100 unchanged selections, 10000 documents / 100 commits: {:?}",
            start.elapsed()
        );
    }
}

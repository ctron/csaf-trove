//! Selects and materializes validation inputs from committed provider snapshots.
use super::{
    git_changes::TreeDiffCache, git_repo::url_to_git_path, processing::ProcessingCheckpoint,
    scratch,
};
use anyhow::{Context, Result};
use git2::{Oid, Repository, TreeWalkMode, TreeWalkResult};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, path::Path, time::Instant};

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
    let started = Instant::now();
    let mut commits_visited = 0u64;
    let mut tree_diffs = TreeDiffCache::default();
    let mut head_id = None;
    let result = (|| -> Result<ProcessingSelection> {
        let repo = Repository::open_bare(repo_path)?;
        let head = repo.head()?.peel_to_commit()?;
        head_id = Some(head.id().to_string());
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
            commits_visited += 1;
            if current.parent_count() != 1 {
                baseline = true;
                break;
            }
            let parent = current.parent(0)?;
            let parent_tree = parent.tree()?;
            let current_tree = current.tree()?;
            let diff = tree_diffs.diff(&repo, Some(&parent_tree), &current_tree)?;
            for delta in diff.iter() {
                for path in [&delta.old_path, &delta.new_path].into_iter().flatten() {
                    changes.insert(path.clone());
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
    })();
    tracing::info!(
        repository = %repo_path.display(),
        checkpoint = previous.map(|checkpoint| checkpoint.commit_id.as_str()),
        head = head_id.as_deref(),
        commits_visited,
        diffs_attempted = tree_diffs.diffs_attempted,
        diff_cache_hits = tree_diffs.cache_hits,
        elapsed_ms = started.elapsed().as_millis(),
        success = result.is_ok(),
        "Processing selection finished"
    );
    result
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
#[cfg(test)]
pub fn materialize_processing(
    repo_path: &Path,
    selection: &ProcessingSelection,
    output: &Path,
) -> Result<()> {
    materialize_processing_with_progress(repo_path, selection, output, |_, _| {})
}

/// Like [`materialize_processing`], but calls `progress(current, total)` per extracted file.
pub fn materialize_processing_with_progress(
    repo_path: &Path,
    selection: &ProcessingSelection,
    output: &Path,
    progress: impl Fn(u64, u64),
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
    let total = paths.len() as u64;
    let mut current = 0u64;
    for path in paths {
        let entry = tree
            .get_path(Path::new(&path))
            .with_context(|| format!("Missing selected input {path}"))?;
        let blob = repo.find_blob(entry.id())?;
        scratch::write(output, Path::new(&path), blob.content())?;
        current += 1;
        progress(current, total);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{
        Storage,
        git_repo::{collect_versions, commit_snapshot, prepare_worktree},
    };
    use git2::{Index, Signature};
    use sea_orm::{ConnectionTrait, DbBackend, Statement};
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

    /// Appends a fixture commit without repeatedly staging a 10,000-entry scratch index.
    fn append_benchmark_commit(
        repo: &Repository,
        index: &mut git2::Index,
        path: &str,
        data: &[u8],
    ) {
        let mut entry = index.get_path(Path::new(path), 0).unwrap();
        entry.id = repo.blob(data).unwrap();
        entry.file_size = data.len() as u32;
        index.add(&entry).unwrap();
        let tree = repo.find_tree(index.write_tree_to(repo).unwrap()).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        let signature = Signature::now("benchmark", "benchmark@example.com").unwrap();
        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            "backlog",
            &tree,
            &[&parent],
        )
        .unwrap();
    }

    /// Measures selection and version recording with a fixed checkpoint and growing retry backlog.
    #[tokio::test]
    #[ignore = "manual performance fixture: 10000 documents and gaps of 0, 100, 1000 commits"]
    async fn benchmark_processing_backlog() {
        for (advisory_changes, reverts) in [(false, false), (true, true), (true, false)] {
            let dir = tempfile::tempdir().unwrap();
            let storage = Storage::new(dir.path()).unwrap();
            let domain = "example.com";
            let repo = storage.repo_path(domain);
            let work = dir.path().join("work");
            let prepared = prepare_worktree(&repo, &work).unwrap();
            fs::create_dir_all(work.join(domain)).unwrap();
            for n in 0..10_000 {
                fs::write(work.join(format!("{domain}/{n}.json")), b"{}").unwrap();
            }
            fs::create_dir_all(work.join("metadata")).unwrap();
            fs::write(
                work.join("metadata/provider-metadata.json"),
                br#"{"last_updated":-1}"#,
            )
            .unwrap();
            commit_snapshot(&prepared, "baseline").unwrap();
            let initial = select_processing(&repo, None, "v1", false).unwrap();
            let db = storage.db.get(domain).await.unwrap();
            db.execute_unprepared(
                "WITH RECURSIVE ids(n) AS (SELECT 0 UNION ALL SELECT n+1 FROM ids WHERE n<9999)
                 INSERT INTO documents (tracking_id, title, url, signature_present, version_count)
                 SELECT CAST(n AS TEXT), 'Document', 'https://example.com/' || n || '.json', 0, 1 FROM ids",
            ).await.unwrap();
            storage.record_versions(domain, None).await.unwrap();
            let git = Repository::open_bare(&repo).unwrap();
            let mut index = Index::new().unwrap();
            index
                .read_tree(&git.head().unwrap().peel_to_tree().unwrap())
                .unwrap();
            let mut committed = 0;
            for gap in [0, 100, 1_000] {
                while committed < gap {
                    let metadata = format!("{{\"last_updated\":{committed}}}");
                    if advisory_changes {
                        // Exercise both repeated transitions and never-before-seen content.
                        let unique = format!("{{\"version\":{committed}}}");
                        let content: &[u8] = if !reverts {
                            unique.as_bytes()
                        } else if committed % 2 == 0 {
                            b"{\"changed\":true}"
                        } else {
                            b"{}"
                        };
                        append_benchmark_commit(&git, &mut index, "example.com/0.json", content);
                    } else {
                        append_benchmark_commit(
                            &git,
                            &mut index,
                            "metadata/provider-metadata.json",
                            metadata.as_bytes(),
                        );
                    }
                    committed += 1;
                }
                for attempt in ["first", "retry"] {
                    let started = Instant::now();
                    let selection =
                        select_processing(&repo, Some(&initial.checkpoint), "v1", false).unwrap();
                    let selection_elapsed = started.elapsed();
                    assert!(!selection.full);
                    assert!(!selection.baseline);
                    assert!(selection.deleted.is_empty());
                    let expected = if advisory_changes && gap > 0 {
                        BTreeSet::from(["example.com/0.json".to_string()])
                    } else {
                        BTreeSet::new()
                    };
                    assert_eq!(selection.advisories, expected);
                    assert_eq!(selection.history, expected);
                    let started = Instant::now();
                    // Time recording independently, even when selection would skip this phase.
                    storage
                        .record_versions(domain, Some(&initial.checkpoint.commit_id))
                        .await
                        .unwrap();
                    let recording_elapsed = started.elapsed();
                    let row = db
                        .query_one_raw(Statement::from_string(
                            DbBackend::Sqlite,
                            "SELECT COUNT(*) AS total FROM document_versions",
                        ))
                        .await
                        .unwrap()
                        .unwrap();
                    let expected_versions = 10_000 + if advisory_changes { gap } else { 0 };
                    assert_eq!(row.try_get::<i64>("", "total").unwrap(), expected_versions);
                    let row = db
                        .query_one_raw(Statement::from_string(
                            DbBackend::Sqlite,
                            "SELECT version_count FROM documents WHERE tracking_id = '0'",
                        ))
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(
                        row.try_get::<i64>("", "version_count").unwrap(),
                        1 + if advisory_changes { gap } else { 0 }
                    );
                    eprintln!(
                        "backlog: documents=10000 advisory_changes={advisory_changes} reverts={reverts} gap={gap} attempt={attempt} selection={selection_elapsed:?} version_recording={recording_elapsed:?}"
                    );
                }
            }
        }
    }
}

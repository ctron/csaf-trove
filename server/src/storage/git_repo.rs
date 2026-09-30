use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    str::from_utf8,
    time::Instant,
};

use super::{
    git_changes::TreeDiffCache,
    git_processing::{advisory_path, path_url},
    scratch,
};
use crate::models::result::{DiffLineInfo, DiffTag};
use anyhow::{Context, Result, ensure};
use git2::{
    BranchType, Delta, ErrorCode, Index, IndexEntry, IndexTime, Mempack, Odb, Oid, PackBuilder,
    Repository, Signature, Tree,
};
use serde::Deserialize;
use walkdir::WalkDir;

/// Opens an existing bare repo or initializes a new one.
pub fn init_bare(path: &Path) -> Result<Repository> {
    if path.exists() {
        Repository::open_bare(path).context("Failed to open bare repo")
    } else {
        Repository::init_bare(path).context("Failed to init bare repo")
    }
}

/// Scratch files and index backed by a provider's existing object database.
#[derive(Debug)]
pub struct PreparedWorktree {
    /// Persistent bare repository containing all objects and history.
    repo_path: PathBuf,
    /// Disposable working directory, including its private `.git/index`.
    worktree_path: PathBuf,
    /// Branch to advance when the sync is committed.
    branch: String,
    /// Branch tip captured before downloading files, or no tip for an initial sync.
    base_commit: Option<Oid>,
}

/// Failures specific to preparing or publishing a sync's Git snapshot.
#[derive(Debug, thiserror::Error)]
enum WorktreeError {
    /// HEAD must identify a local branch that can receive new commits.
    #[error("Provider repository HEAD must point to a local branch")]
    InvalidHead,
    /// Another writer changed the branch after preparation.
    #[error("Provider branch {0} changed during sync; refusing to overwrite it")]
    BranchChanged(String),
}

/// Resolves HEAD, repairing a missing branch using the first existing local branch.
fn resolve_branch(repo: &Repository) -> Result<(String, Option<Oid>)> {
    let head = repo.find_reference("HEAD")?;
    let branch = head
        .symbolic_target()?
        .filter(|name| name.starts_with("refs/heads/"))
        .ok_or(WorktreeError::InvalidHead)?;
    match repo.find_reference(branch) {
        Ok(reference) => return Ok((branch.to_owned(), Some(reference.peel_to_commit()?.id()))),
        Err(error) if error.code() == ErrorCode::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if let Some(entry) = repo.branches(Some(BranchType::Local))?.next() {
        let (fallback, _) = entry?;
        let reference = fallback.get();
        let name = reference.name()?;
        let oid = reference.peel_to_commit()?.id();
        repo.set_head(name)?;
        tracing::info!("Fixed bare repo HEAD → {name}");
        return Ok((name.to_owned(), Some(oid)));
    }
    Ok((branch.to_owned(), None))
}

/// Attaches scratch paths only to this repository handle, without changing config.
fn open_worktree(worktree: &PreparedWorktree) -> Result<Repository> {
    let repo = Repository::open_bare(&worktree.repo_path)?;
    repo.set_workdir(&worktree.worktree_path, false)?;
    let mut index = Index::open(&worktree.worktree_path.join(".git/index"))?;
    repo.set_index(&mut index)?;
    Ok(repo)
}

/// Prepares a private index without copying objects or transferring Git history.
///
/// Scratch always starts empty: the index carries HEAD, so files absent from scratch keep
/// their committed content and only downloaded files need to be written.
/// The caller must hold the provider's pipeline lock until committing and cleanup.
pub fn prepare_worktree(repo_path: &Path, worktree_path: &Path) -> Result<PreparedWorktree> {
    let started = Instant::now();
    let repo = init_bare(repo_path)?;
    let (branch, base_commit) = resolve_branch(&repo)?;
    if worktree_path.exists() {
        fs::remove_dir_all(worktree_path).context("Failed to remove previous scratch worktree")?;
    }
    fs::create_dir_all(worktree_path.join(".git"))?;
    let worktree = PreparedWorktree {
        repo_path: fs::canonicalize(repo_path)?,
        worktree_path: fs::canonicalize(worktree_path)?,
        branch,
        base_commit,
    };
    let repo = open_worktree(&worktree)?;
    let mut index = repo.index()?;
    if let Some(oid) = base_commit {
        let tree = repo.find_commit(oid)?.tree()?;
        index.read_tree(&tree)?;
    }
    index.write()?;
    tracing::info!(
        repository = %repo_path.display(),
        entries = index.len(),
        elapsed_ms = started.elapsed().as_millis(),
        "Prepared worktree using existing Git objects"
    );
    Ok(worktree)
}

/// Stages scratch files and commits directly into the persistent bare repository.
///
/// Decompresses advisories individually into Git under their original paths. Existing
/// index entries are preserved for files absent from incremental scratch directories.
/// Returns `false` if nothing changed.
#[cfg(test)]
pub fn commit_all(worktree: &PreparedWorktree, message: &str) -> Result<bool> {
    commit_all_with_progress(worktree, message, u64::MAX, |_, _| {})
}

/// Like [`commit_all`], but calls `progress(current, total)` after staging each file.
///
/// `pack_threshold` controls how many bytes of blob data are buffered in memory
/// before flushing to a packfile. Pass `u64::MAX` to flush once at the end.
pub fn commit_all_with_progress(
    worktree: &PreparedWorktree,
    message: &str,
    pack_threshold: u64,
    progress: impl Fn(u64, u64),
) -> Result<bool> {
    let worktree_path = &worktree.worktree_path;
    // Index::open would silently create an empty index if scratch data was lost.
    fs::metadata(worktree_path.join(".git/index")).context("Missing prepared worktree index")?;
    {
        let repo = open_worktree(worktree)?;
        let current = match repo.find_reference(&worktree.branch) {
            Ok(reference) => Some(reference.peel_to_commit()?.id()),
            Err(error) if error.code() == ErrorCode::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        if current != worktree.base_commit {
            return Err(WorktreeError::BranchChanged(worktree.branch.clone()).into());
        }
        let mut index = repo.index()?;

        let odb = repo.odb()?;
        let mempack = odb.add_new_mempack_backend(1000)?;

        // Downloaded key sets replace the old set, including keys removed by rotation.
        if worktree_path.join("metadata/keys").is_dir() {
            let keys: Vec<_> = index
                .iter()
                .filter(|entry| entry.path.starts_with(b"metadata/keys/"))
                .map(|entry| entry.path)
                .collect();
            for key in keys {
                index.remove_path(Path::new(from_utf8(&key)?))?;
            }
        }

        let git_dir = worktree_path.join(".git");
        let total = WalkDir::new(worktree_path)
            .into_iter()
            .filter_entry(|e| e.path() != git_dir)
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .count() as u64;

        let mut staged = 0u64;
        let mut buffered_bytes = 0u64;
        let mut buffered_objects = Vec::new();
        let git_dir = worktree_path.join(".git");
        for entry in WalkDir::new(worktree_path)
            .into_iter()
            .filter_entry(|e| e.path() != git_dir)
        {
            let entry = entry.context("failed to walk worktree")?;
            if entry.file_type().is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(worktree_path)
                    .context("file is not under worktree")?;
                let logical = scratch::logical_path(relative);
                if scratch::is_advisory(&logical) && !logical.starts_with("metadata") {
                    for suffix in ["asc", "sha256", "sha512"] {
                        let sidecar = logical.with_added_extension(suffix);
                        if !worktree_path.join(&sidecar).exists()
                            && index.get_path(&sidecar, 0).is_some()
                        {
                            index.remove_path(&sidecar)?;
                        }
                    }
                }
                if logical != relative {
                    ensure!(
                        !worktree_path.join(&logical).exists(),
                        "Both plain and compressed scratch files exist: {}",
                        logical.display()
                    );
                    let data = scratch::read(entry.path())?;
                    buffered_bytes += data.len() as u64;
                    let entry = index.get_path(&logical, 0).unwrap_or(IndexEntry {
                        ctime: IndexTime::new(0, 0),
                        mtime: IndexTime::new(0, 0),
                        dev: 0,
                        ino: 0,
                        mode: 0o100644,
                        uid: 0,
                        gid: 0,
                        file_size: 0,
                        id: Oid::ZERO_SHA1,
                        flags: 0,
                        flags_extended: 0,
                        path: logical
                            .to_str()
                            .context("Invalid scratch path")?
                            .as_bytes()
                            .to_vec(),
                    });
                    index.add_frombuffer(&entry, &data)?;
                } else {
                    buffered_bytes += entry.metadata()?.len();
                    index.add_path(relative)?;
                }
                buffered_objects.push(
                    index
                        .get_path(&logical, 0)
                        .context("Missing staged index entry")?
                        .id,
                );
                staged += 1;
                progress(staged, total);

                if buffered_bytes >= pack_threshold {
                    flush_mempack(&mempack, &repo, &odb, &buffered_objects)?;
                    buffered_objects.clear();
                    buffered_bytes = 0;
                }
            }
        }
        flush_mempack(&mempack, &repo, &odb, &buffered_objects)?;
        index.write()?;
    }

    // Only blobs use the bounded mempack. Reopen without that backend so trees and
    // commits go directly to disk instead of recursively repacking the whole snapshot.
    let repo = open_worktree(worktree)?;
    let tree_oid = repo.index()?.write_tree()?;
    let tree = repo.find_tree(tree_oid)?;

    let sig = Signature::now("csaf-trove", "csaf-trove@localhost")?;

    let parent = worktree
        .base_commit
        .map(|oid| repo.find_commit(oid))
        .transpose()?;

    if let Some(ref parent) = parent
        && parent.tree()?.id() == tree_oid
    {
        tracing::debug!("No changes to commit for {}", worktree.repo_path.display());
        return Ok(false);
    }

    let parents: Vec<&git2::Commit> = parent.as_ref().map(|p| vec![p]).unwrap_or_default();
    let oid = repo.commit(None, &sig, &sig, message, &tree, &parents)?;

    match repo.reference_matching(
        &worktree.branch,
        oid,
        true,
        worktree.base_commit.unwrap_or(Oid::ZERO_SHA1),
        message,
    ) {
        Ok(_) => {}
        Err(error) if error.code() == ErrorCode::Modified || error.code() == ErrorCode::Exists => {
            return Err(WorktreeError::BranchChanged(worktree.branch.clone()).into());
        }
        Err(error) => return Err(error).context("Failed to publish provider commit"),
    }

    Ok(true)
}

/// Streams a bounded batch of staged blobs to disk before resetting the mempack.
fn flush_mempack(
    mempack: &Mempack<'_>,
    repo: &Repository,
    odb: &Odb<'_>,
    objects: &[Oid],
) -> Result<()> {
    if objects.is_empty() {
        return Ok(());
    }
    let started = Instant::now();
    // Mempack::dump only walks commits. Insert each staged blob explicitly, without
    // following references to any objects already persisted in earlier batches.
    let mut builder = repo.packbuilder()?;
    for &oid in objects {
        builder.insert_object(oid, None)?;
    }
    let mut packwriter = odb.packwriter()?;
    let bytes = stream_pack(&mut builder, &mut packwriter)?;
    packwriter.commit()?;
    odb.refresh()?;
    mempack.reset()?;
    tracing::info!(
        repository = %repo.path().display(),
        objects = builder.object_count(),
        bytes,
        elapsed_ms = started.elapsed().as_millis(),
        "Persisted Git blob batch"
    );
    Ok(())
}

/// Streams pack chunks to a writer, preserving the original error if writing fails.
fn stream_pack(builder: &mut PackBuilder<'_>, writer: &mut impl Write) -> Result<u64> {
    let mut write_error = None;
    let mut bytes = 0;
    let result = builder.foreach(|chunk| match writer.write_all(chunk) {
        Ok(()) => {
            bytes += chunk.len() as u64;
            true
        }
        Err(error) => {
            write_error = Some(error);
            false
        }
    });
    if let Some(error) = write_error {
        return Err(error).context("Failed to stream Git pack");
    }
    result?;
    Ok(bytes)
}

/// Snapshot published by a sync commit, including unchanged runs.
pub struct CommitOutcome {
    /// Resulting immutable Git commit.
    pub commit_id: String,
}

/// Commits downloaded files and returns the resulting snapshot identity.
#[cfg(test)]
pub fn commit_snapshot(worktree: &PreparedWorktree, message: &str) -> Result<CommitOutcome> {
    commit_snapshot_with_progress(worktree, message, u64::MAX, |_, _| {})
}

/// Like [`commit_snapshot`], but reports staging progress via the callback.
pub fn commit_snapshot_with_progress(
    worktree: &PreparedWorktree,
    message: &str,
    pack_threshold: u64,
    progress: impl Fn(u64, u64),
) -> Result<CommitOutcome> {
    commit_all_with_progress(worktree, message, pack_threshold, progress)?;
    let repo = Repository::open_bare(&worktree.repo_path)?;
    Ok(CommitOutcome {
        commit_id: repo.head()?.peel_to_commit()?.id().to_string(),
    })
}

/// Reads a blob from the HEAD commit of a bare repo at the given tree path.
///
/// Returns `None` if the repo has no HEAD or the path does not exist in the tree.
pub fn read_head_blob(repo_path: &Path, tree_path: &str) -> Result<Option<Vec<u8>>> {
    let repo = Repository::open_bare(repo_path)?;
    let Ok(head) = repo.head() else {
        return Ok(None);
    };
    let commit = head.peel_to_commit()?;
    let tree = commit.tree()?;
    let Some(oid) = blob_oid_at_path(&tree, tree_path)? else {
        return Ok(None);
    };
    let blob = repo.find_blob(oid)?;
    Ok(Some(blob.content().to_vec()))
}

/// Looks up a blob OID at a known path within a commit's tree.
fn blob_oid_at_path(tree: &Tree<'_>, path: &str) -> Result<Option<Oid>> {
    match tree.get_path(std::path::Path::new(path)) {
        Ok(entry) => Ok(Some(entry.id())),
        Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Converts a document URL to its git tree path: `<domain>/<url_path>`.
///
/// The worktree (and thus the git tree) stores files as
/// `<domain>/<url_path>`, mirroring the URL structure directly.
pub(super) fn url_to_git_path(url: &str) -> Result<Option<String>> {
    let parsed = url::Url::parse(url).context("invalid document URL")?;
    let domain = match parsed.host_str() {
        Some(d) => d,
        None => return Ok(None),
    };
    let path = parsed.path().trim_start_matches('/');
    if path.is_empty() {
        return Ok(None);
    }
    Ok(Some(format!("{domain}/{path}")))
}

/// CSAF tracking fields captured for each recorded document version.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackingSummary {
    /// `document.tracking.status`.
    pub status: Option<String>,
    /// `document.tracking.version`.
    pub version: Option<String>,
    /// `document.tracking.current_release_date`.
    pub current_release_date: Option<String>,
}

/// Minimal CSAF envelope, deserialized without building a full JSON tree.
#[derive(Default, Deserialize)]
struct CsafEnvelope {
    /// The `document` section.
    #[serde(default)]
    document: CsafDocument,
}

/// The parts of the CSAF `document` section needed for version records.
#[derive(Default, Deserialize)]
struct CsafDocument {
    /// The `tracking` section.
    #[serde(default)]
    tracking: CsafTracking,
}

/// Raw tracking values, kept loose so malformed types do not discard the rest.
#[derive(Default, Deserialize)]
struct CsafTracking {
    /// Raw `status` value.
    #[serde(default)]
    status: Option<serde_json::Value>,
    /// Raw `version` value.
    #[serde(default)]
    version: Option<serde_json::Value>,
    /// Raw `current_release_date` value.
    #[serde(default)]
    current_release_date: Option<serde_json::Value>,
}

/// Renders a scalar JSON value as text, ignoring objects, arrays and nulls.
fn scalar_text(value: Option<serde_json::Value>) -> Option<String> {
    match value? {
        serde_json::Value::String(s) => Some(s),
        serde_json::Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Extracts tracking fields from a CSAF document; invalid JSON yields empty fields.
pub fn parse_tracking(blob: &[u8]) -> TrackingSummary {
    let tracking = serde_json::from_slice::<CsafEnvelope>(blob)
        .unwrap_or_default()
        .document
        .tracking;
    TrackingSummary {
        status: scalar_text(tracking.status),
        version: scalar_text(tracking.version),
        current_release_date: scalar_text(tracking.current_release_date),
    }
}

/// A document whose content changed in a commit.
#[derive(Debug, Clone)]
pub struct ChangedDocument {
    /// Canonical advisory URL.
    pub url: String,
    /// Blob OID of the new content.
    pub blob_id: String,
    /// Tracking fields of the new content.
    pub tracking: TrackingSummary,
}

/// Document versions introduced by a single commit.
#[derive(Debug, Clone)]
pub struct CommitVersions {
    /// Commit SHA.
    pub commit_id: String,
    /// Commit timestamp as Unix seconds.
    pub timestamp: i64,
    /// Commit message.
    pub message: String,
    /// Advisories added or modified by this commit.
    pub documents: Vec<ChangedDocument>,
}

/// Emits advisory versions per commit, oldest first, following first parents from HEAD.
///
/// With `since`, only commits after that commit are visited; if it is not a
/// first-parent ancestor of HEAD, the whole history is visited. Each change is
/// detected by diffing a commit against its parent, so a version is recorded
/// whenever the content differs from the previous commit, including reverts
/// and re-additions after deletion.
pub fn collect_versions(
    repo_path: &Path,
    since: Option<&str>,
    mut emit: impl FnMut(CommitVersions) -> Result<()>,
) -> Result<()> {
    let repo = Repository::open_bare(repo_path)?;
    let Ok(head) = repo.head() else {
        return Ok(());
    };
    let since = since.map(Oid::from_str).transpose()?;

    let mut commits = Vec::new();
    let mut current = Some(head.peel_to_commit()?);
    while let Some(commit) = current {
        if Some(commit.id()) == since {
            break;
        }
        current = commit.parents().next();
        commits.push(commit);
    }

    let mut tree_diffs = TreeDiffCache::default();
    for commit in commits.into_iter().rev() {
        let tree = commit.tree()?;
        let parent_tree = commit.parents().next().map(|p| p.tree()).transpose()?;
        let diff = tree_diffs.diff(&repo, parent_tree.as_ref(), &tree)?;
        let mut documents = Vec::new();
        for delta in diff.iter() {
            if !matches!(delta.status, Delta::Added | Delta::Modified) {
                continue;
            }
            let Some(path) = delta.new_path.as_deref() else {
                continue;
            };
            if advisory_path(path).as_deref() != Some(path) {
                continue;
            }
            let blob = repo.find_blob(delta.new_id)?;
            documents.push(ChangedDocument {
                url: path_url(path),
                blob_id: delta.new_id.to_string(),
                tracking: parse_tracking(blob.content()),
            });
        }
        if !documents.is_empty() {
            emit(CommitVersions {
                commit_id: commit.id().to_string(),
                timestamp: commit.time().seconds(),
                message: commit.message().unwrap_or("").to_string(),
                documents,
            })?;
        }
    }
    Ok(())
}

/// Reads a blob by OID, returning `None` if it does not exist.
pub fn read_blob(repo_path: &Path, blob_id: &str) -> Result<Option<Vec<u8>>> {
    let repo = Repository::open_bare(repo_path)?;
    let oid = Oid::from_str(blob_id).context("invalid blob ID")?;
    match repo.find_blob(oid) {
        Ok(blob) => Ok(Some(blob.content().to_vec())),
        Err(e) if e.code() == ErrorCode::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Computes a line-level diff between two document contents.
///
/// Both blobs are pretty-printed as JSON to normalize formatting. If the
/// pretty-printed content is identical (formatting-only change), falls back
/// to diffing the raw content so the actual differences remain visible.
pub fn diff_documents(old_blob: &[u8], new_blob: &[u8]) -> Vec<DiffLineInfo> {
    let old_pretty = pretty_print_or_raw(old_blob);
    let new_pretty = pretty_print_or_raw(new_blob);

    let (old_text, new_text) = if old_pretty == new_pretty {
        let old_raw = String::from_utf8_lossy(old_blob).into_owned();
        let new_raw = String::from_utf8_lossy(new_blob).into_owned();
        (old_raw, new_raw)
    } else {
        (old_pretty, new_pretty)
    };

    let diff = similar::TextDiff::from_lines(&old_text, &new_text);

    diff.iter_all_changes()
        .map(|change| {
            let tag = match change.tag() {
                similar::ChangeTag::Equal => DiffTag::Equal,
                similar::ChangeTag::Insert => DiffTag::Insert,
                similar::ChangeTag::Delete => DiffTag::Delete,
            };
            DiffLineInfo {
                tag,
                content: change.value().trim_end_matches('\n').to_string(),
            }
        })
        .collect()
}

/// Attempts to parse bytes as JSON and pretty-print; falls back to raw UTF-8.
fn pretty_print_or_raw(blob: &[u8]) -> String {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(blob) {
        serde_json::to_string_pretty(&value)
            .unwrap_or_else(|_| String::from_utf8_lossy(blob).into_owned())
    } else {
        String::from_utf8_lossy(blob).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeSet, fs, io, path::PathBuf};

    /// Lists pack and index filenames to isolate objects written by an incremental sync.
    fn pack_files(repo: &Path) -> BTreeSet<PathBuf> {
        fs::read_dir(repo.join("objects/pack"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into())
            .collect()
    }

    /// An incremental batch must contain only downloaded blobs, not the previous snapshot.
    #[test]
    fn incremental_commit_does_not_repack_unchanged_blobs() {
        for threshold in [1, u64::MAX] {
            let dir = tempfile::tempdir().unwrap();
            let bare_path = dir.path().join("repo.git");
            let work_path = dir.path().join("work");
            let prepared = prepare_worktree(&bare_path, &work_path).unwrap();
            for i in 0..256 {
                scratch::write(
                    &work_path,
                    Path::new(&format!("example.com/advisories/{i}.json")),
                    format!("document {i}").as_bytes(),
                )
                .unwrap();
            }
            assert!(commit_all(&prepared, "initial").unwrap());
            let initial = Repository::open_bare(&bare_path)
                .unwrap()
                .head()
                .unwrap()
                .target()
                .unwrap();
            let before = pack_files(&bare_path);
            let prepared = prepare_worktree(&bare_path, &work_path).unwrap();
            scratch::write(
                &work_path,
                Path::new("example.com/advisories/0.json"),
                b"updated",
            )
            .unwrap();
            assert!(commit_all_with_progress(&prepared, "updated", threshold, |_, _| {}).unwrap());

            // Open only the new packs in an independent ODB; old packs cannot mask a
            // regression that repacks the entire snapshot during the final flush.
            let isolated_repo = Repository::init_bare(dir.path().join("isolated.git")).unwrap();
            let isolated = isolated_repo.path().join("objects");
            fs::create_dir_all(isolated.join("pack")).unwrap();
            let after = pack_files(&bare_path);
            let added: Vec<_> = after.difference(&before).collect();
            assert_eq!(added.len(), 2, "one blob pack and its index");
            for file in added {
                fs::copy(
                    bare_path.join("objects/pack").join(file),
                    isolated.join("pack").join(file),
                )
                .unwrap();
            }
            let odb = isolated_repo.odb().unwrap();
            let mut objects = Vec::new();
            odb.foreach(|oid| {
                objects.push(*oid);
                true
            })
            .unwrap();
            assert_eq!(objects.len(), 1);
            assert_eq!(odb.read(objects[0]).unwrap().data(), b"updated");

            let repo = Repository::open_bare(&bare_path).unwrap();
            let head = repo.head().unwrap().peel_to_commit().unwrap();
            assert_eq!(head.parent_id(0).unwrap(), initial);
            for (commit, updated) in [(initial, false), (head.id(), true)] {
                let tree = repo.find_commit(commit).unwrap().tree().unwrap();
                for i in 0..256 {
                    let path = format!("example.com/advisories/{i}.json");
                    let blob = repo
                        .find_blob(tree.get_path(Path::new(&path)).unwrap().id())
                        .unwrap();
                    let expected = if updated && i == 0 {
                        "updated".to_owned()
                    } else {
                        format!("document {i}")
                    };
                    assert_eq!(blob.content(), expected.as_bytes());
                }
            }
        }
    }

    /// Empty staging batches produce no packfiles, including an empty initial snapshot.
    #[test]
    fn empty_commit_does_not_write_blob_pack() {
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("repo.git");
        let work = dir.path().join("work");
        let prepared = prepare_worktree(&bare, &work).unwrap();
        assert!(commit_all(&prepared, "empty initial").unwrap());
        assert!(pack_files(&bare).is_empty());
        let prepared = prepare_worktree(&bare, &work).unwrap();
        assert!(!commit_all(&prepared, "unchanged").unwrap());
        assert!(pack_files(&bare).is_empty());
    }

    /// A destination that fails immediately with a recognizable original I/O error.
    struct FailingWriter;

    impl Write for FailingWriter {
        /// Simulates a failed pack write.
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "pack write denied",
            ))
        }

        /// No buffered data needs flushing.
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Callback cancellation must not replace the underlying destination write error.
    #[test]
    fn streaming_pack_preserves_write_error() {
        let (_dir, bare) = create_test_repo(&[("doc.json", b"initial")]);
        let repo = Repository::open_bare(&bare).unwrap();
        let original = repo.head().unwrap().target().unwrap();
        let odb = repo.odb().unwrap();
        let _mempack = odb.add_new_mempack_backend(1000).unwrap();
        let oid = odb.write(git2::ObjectType::Blob, b"pending").unwrap();
        let mut builder = repo.packbuilder().unwrap();
        builder.insert_object(oid, None).unwrap();
        let error = stream_pack(&mut builder, &mut FailingWriter).unwrap_err();
        let error = error.downcast_ref::<io::Error>().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(error.to_string(), "pack write denied");
        assert_eq!(odb.read(oid).unwrap().data(), b"pending");
        assert_eq!(repo.head().unwrap().target().unwrap(), original);
    }

    /// Creates provider history through the production setup and commit path.
    fn create_test_repo(files: &[(&str, &[u8])]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let bare_path = dir.path().join("repo.git");
        let work_path = dir.path().join("work");
        let prepared = prepare_worktree(&bare_path, &work_path).unwrap();
        for (path, content) in files {
            let full = work_path.join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(&full, content).unwrap();
        }
        assert!(commit_all(&prepared, "initial").unwrap());
        (dir, bare_path)
    }

    /// Threshold flushes preserve plain and compressed blobs across incremental commits.
    #[test]
    fn mempack_threshold_flush_preserves_objects() {
        for threshold in [1, 16, u64::MAX] {
            let dir = tempfile::tempdir().unwrap();
            let bare_path = dir.path().join("repo.git");
            let work_path = dir.path().join("work");
            for revision in ["initial", "updated"] {
                let prepared = prepare_worktree(&bare_path, &work_path).unwrap();
                scratch::write(
                    &work_path,
                    Path::new("example.com/advisories/rhsa-2022_5775.json"),
                    revision.as_bytes(),
                )
                .unwrap();
                fs::write(work_path.join("plain.txt"), revision).unwrap();
                if revision == "initial" {
                    fs::write(work_path.join("preserved.txt"), b"unchanged").unwrap();
                }
                assert!(
                    commit_all_with_progress(&prepared, revision, threshold, |_, _| {}).unwrap()
                );
                for path in ["example.com/advisories/rhsa-2022_5775.json", "plain.txt"] {
                    assert_eq!(
                        read_head_blob(&bare_path, path).unwrap().unwrap(),
                        revision.as_bytes()
                    );
                }
                assert_eq!(
                    read_head_blob(&bare_path, "preserved.txt")
                        .unwrap()
                        .unwrap(),
                    b"unchanged"
                );
                let prepared = prepare_worktree(&bare_path, &work_path).unwrap();
                fs::write(work_path.join("plain.txt"), revision).unwrap();
                assert!(
                    !commit_all_with_progress(&prepared, "unchanged", threshold, |_, _| {})
                        .unwrap()
                );
            }
        }
    }

    /// Verifies that scratch state never duplicates objects or alters bare config.
    #[test]
    fn worktree_setup_reuses_objects_and_recovers_scratch() {
        let (dir, bare_path) = create_test_repo(&[("example.com/doc.json", b"old")]);
        let config = fs::read(bare_path.join("config")).unwrap();
        let head = fs::read(bare_path.join("HEAD")).unwrap();
        let work_path = dir.path().join("work");
        // Simulate a legacy clone left behind by a killed process.
        fs::create_dir_all(work_path.join(".git/objects/pack")).unwrap();
        fs::write(work_path.join(".git/objects/pack/abandoned.pack"), b"old").unwrap();
        let prepared = prepare_worktree(&bare_path, &work_path).unwrap();
        assert!(!work_path.join("example.com").exists());
        assert!(!work_path.join(".git/objects").exists());
        assert_eq!(fs::read_dir(work_path.join(".git")).unwrap().count(), 1);
        assert_eq!(Index::open(&work_path.join(".git/index")).unwrap().len(), 1);
        assert!(!commit_all(&prepared, "no changes").unwrap());
        assert_eq!(fs::read(bare_path.join("config")).unwrap(), config);
        assert_eq!(fs::read(bare_path.join("HEAD")).unwrap(), head);
        assert!(!bare_path.join("index").exists());
        assert!(Repository::open_bare(&bare_path).unwrap().is_bare());
        fs::remove_dir_all(&work_path).unwrap();
        assert_eq!(
            read_head_blob(&bare_path, "example.com/doc.json")
                .unwrap()
                .unwrap(),
            b"old"
        );
    }

    /// Syncs keep unchanged history without materializing it and commit original bytes.
    #[test]
    fn worktree_keeps_history_without_materializing() {
        let (dir, bare_path) = create_test_repo(&[("example.com/doc.json", b"old")]);
        let work_path = dir.path().join("full");
        let prepared = prepare_worktree(&bare_path, &work_path).unwrap();
        assert!(!work_path.join("example.com").exists());
        assert!(!commit_all(&prepared, "unchanged").unwrap());
        scratch::write(&work_path, Path::new("example.com/doc.json"), b"new").unwrap();
        assert!(commit_all(&prepared, "changed").unwrap());
        assert_eq!(
            read_head_blob(&bare_path, "example.com/doc.json")
                .unwrap()
                .unwrap(),
            b"new"
        );
    }

    /// Empty repositories retain their configured branch name for the first commit.
    #[test]
    fn initial_sync_supports_custom_branch() {
        let dir = tempfile::tempdir().unwrap();
        let bare_path = dir.path().join("repo.git");
        let bare = init_bare(&bare_path).unwrap();
        bare.set_head("refs/heads/provider/history").unwrap();
        let work_path = dir.path().join("work");
        let prepared = prepare_worktree(&bare_path, &work_path).unwrap();
        assert_eq!(prepared.base_commit, None);
        fs::write(work_path.join("doc.json"), b"initial").unwrap();
        assert!(commit_all(&prepared, "initial").unwrap());
        assert_eq!(
            bare.head().unwrap().name().unwrap(),
            "refs/heads/provider/history"
        );
        assert_eq!(
            bare.head()
                .unwrap()
                .peel_to_commit()
                .unwrap()
                .parent_count(),
            0
        );
    }

    /// A missing HEAD branch falls back without losing existing history.
    #[test]
    fn worktree_repairs_missing_head_branch() {
        let (dir, bare_path) = create_test_repo(&[("doc.json", b"old")]);
        let bare = Repository::open_bare(&bare_path).unwrap();
        let original = bare.head().unwrap().name().unwrap().to_owned();
        let oid = bare.head().unwrap().target().unwrap();
        bare.set_head("refs/heads/missing").unwrap();
        let prepared = prepare_worktree(&bare_path, &dir.path().join("work")).unwrap();
        assert_eq!(prepared.branch, original);
        assert_eq!(prepared.base_commit, Some(oid));
        assert_eq!(bare.head().unwrap().target(), Some(oid));
    }

    /// Corrupt objects must fail setup rather than start a new history.
    #[test]
    fn worktree_propagates_missing_commit_object() {
        let (dir, bare_path) = create_test_repo(&[("doc.json", b"old")]);
        let oid = Repository::open_bare(&bare_path)
            .unwrap()
            .head()
            .unwrap()
            .target()
            .unwrap()
            .to_string();
        fs::remove_file(bare_path.join("objects").join(&oid[..2]).join(&oid[2..])).unwrap();
        assert!(prepare_worktree(&bare_path, &dir.path().join("work")).is_err());
    }

    /// Competing updates are rejected for both existing and initially absent branches.
    #[test]
    fn direct_commit_rejects_branch_changes() {
        for initial in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let bare_path = dir.path().join("repo.git");
            let work_path = dir.path().join("work");
            let mut prepared = prepare_worktree(&bare_path, &work_path).unwrap();
            if !initial {
                fs::write(work_path.join("doc.json"), b"old").unwrap();
                commit_all(&prepared, "initial").unwrap();
                prepared = prepare_worktree(&bare_path, &work_path).unwrap();
            }
            let competing_path = dir.path().join("competing");
            let competing = prepare_worktree(&bare_path, &competing_path).unwrap();
            fs::write(competing_path.join("doc.json"), b"competing").unwrap();
            commit_all(&competing, "competing").unwrap();
            fs::write(work_path.join("doc.json"), b"stale").unwrap();
            let error = commit_all(&prepared, "stale").unwrap_err();
            assert!(matches!(
                error.downcast_ref::<WorktreeError>(),
                Some(WorktreeError::BranchChanged(_))
            ));
            assert_eq!(
                read_head_blob(&bare_path, "doc.json").unwrap().unwrap(),
                b"competing"
            );
        }
    }

    /// Publication rejects a competing branch update after the initial staging check.
    #[test]
    fn direct_commit_rejects_branch_changes_during_staging() {
        let (dir, bare_path) = create_test_repo(&[("doc.json", b"old")]);
        let repo = Repository::open_bare(&bare_path).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        let signature = Signature::now("test", "test@example.com").unwrap();
        let competing = repo
            .commit(
                None,
                &signature,
                &signature,
                "competing",
                &parent.tree().unwrap(),
                &[&parent],
            )
            .unwrap();
        let work = dir.path().join("work");
        let prepared = prepare_worktree(&bare_path, &work).unwrap();
        fs::write(work.join("doc.json"), b"stale").unwrap();
        let error = commit_all_with_progress(&prepared, "stale", 1, |_, _| {
            repo.reference(&prepared.branch, competing, true, "competing update")
                .unwrap();
        })
        .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<WorktreeError>(),
            Some(WorktreeError::BranchChanged(_))
        ));
        assert_eq!(repo.head().unwrap().target().unwrap(), competing);
        assert_eq!(
            read_head_blob(&bare_path, "doc.json").unwrap().unwrap(),
            b"old"
        );
    }

    /// A lost index cannot silently discard the provider's existing documents.
    #[test]
    fn direct_commit_rejects_missing_index() {
        let (dir, bare_path) = create_test_repo(&[("doc.json", b"old")]);
        let work_path = dir.path().join("work");
        let prepared = prepare_worktree(&bare_path, &work_path).unwrap();
        fs::remove_file(work_path.join(".git/index")).unwrap();
        assert!(commit_all(&prepared, "lost index").is_err());
        assert_eq!(
            read_head_blob(&bare_path, "doc.json").unwrap().unwrap(),
            b"old"
        );
    }

    /// Measures incremental setup on a disposable repository snapshot, without committing.
    /// Set CSAF_TROVE_BENCH_REPO to the snapshot and CSAF_TROVE_BENCH_MODE to direct or fetch.
    /// Run each mode in a separate process to measure peak RSS using /usr/bin/time -v.
    #[test]
    #[ignore = "requires a disposable provider repository snapshot"]
    fn benchmark_worktree_setup() -> Result<()> {
        let repo_path = PathBuf::from(std::env::var_os("CSAF_TROVE_BENCH_REPO").unwrap());
        let mode = std::env::var("CSAF_TROVE_BENCH_MODE").unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let work_path = scratch.path().join("work");
        let started = Instant::now();
        let entries = match mode.as_str() {
            "direct" => {
                let prepared = prepare_worktree(&repo_path, &work_path).unwrap();
                assert!(!work_path.join(".git/objects").exists());
                let count = open_worktree(&prepared).unwrap().index().unwrap().len();
                assert_eq!(fs::read_dir(&work_path).unwrap().count(), 1);
                count
            }
            "fetch" => {
                let repo = Repository::init(&work_path).unwrap();
                repo.remote("origin", repo_path.to_str().unwrap())
                    .unwrap()
                    .fetch(&["refs/heads/*:refs/remotes/origin/*"], None, None)
                    .unwrap();
                let bare = Repository::open_bare(&repo_path).unwrap();
                let oid = bare.head().unwrap().target().unwrap();
                let mut index = repo.index().unwrap();
                index
                    .read_tree(&repo.find_commit(oid).unwrap().tree().unwrap())
                    .unwrap();
                index.write().unwrap();
                index.len()
            }
            _ => anyhow::bail!("unknown benchmark mode: {mode}"),
        };
        eprintln!(
            "mode={mode} entries={entries} elapsed={:?}",
            started.elapsed()
        );
        Ok(())
    }

    #[test]
    fn read_head_blob_returns_content() {
        let (_dir, bare_path) =
            create_test_repo(&[("metadata/provider-metadata.json", b"{\"distributions\":[]}")]);
        let result = read_head_blob(&bare_path, "metadata/provider-metadata.json").unwrap();
        assert_eq!(result, Some(b"{\"distributions\":[]}".to_vec()));
    }

    #[test]
    fn read_head_blob_returns_none_for_missing() {
        let (_dir, bare_path) = create_test_repo(&[("example.com/doc.json", b"{}")]);
        let result = read_head_blob(&bare_path, "metadata/provider-metadata.json").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn url_to_git_path_converts_url() {
        let path = url_to_git_path(
            "https://security.access.redhat.com/data/csaf/v2/advisories/2024/rhsa-2024_1234.json",
        )
        .unwrap();
        assert_eq!(
            path,
            Some(
                "security.access.redhat.com/data/csaf/v2/advisories/2024/rhsa-2024_1234.json"
                    .to_string()
            )
        );
    }

    #[test]
    fn url_to_git_path_returns_none_for_empty_path() {
        let path = url_to_git_path("https://example.com").unwrap();
        assert_eq!(path, None);
    }

    /// Counts blob entries in a tree recursively.
    fn count_tree_blobs(repo: &Repository, tree: &Tree) -> usize {
        let mut count = 0;
        for entry in tree.iter() {
            match entry.kind() {
                Some(git2::ObjectType::Blob) => count += 1,
                Some(git2::ObjectType::Tree) => {
                    if let Ok(sub) = repo.find_tree(entry.id()) {
                        count += count_tree_blobs(repo, &sub);
                    }
                }
                _ => {}
            }
        }
        count
    }

    #[test]
    fn commit_all_preserves_existing_index_entries() {
        let files: &[(&str, &[u8])] = &[
            ("example.com/advisories/2024/adv-001.json", b"{\"a\":1}"),
            ("example.com/advisories/2024/adv-002.json", b"{\"a\":2}"),
            ("example.com/advisories/2025/adv-003.json", b"{\"a\":3}"),
        ];
        let (_dir, bare_path) = create_test_repo(files);

        let inc_work = _dir.path().join("incremental");
        let prepared = prepare_worktree(&bare_path, &inc_work).unwrap();

        // Add one new file to the working directory (simulating incremental sync).
        scratch::write(
            &inc_work,
            Path::new("example.com/advisories/2025/adv-004.json"),
            b"{\"a\":4}",
        )
        .unwrap();

        // commit_all must preserve the 3 existing index entries.
        let changed = commit_all(&prepared, "incremental").unwrap();
        assert!(changed, "should detect changes");

        // Verify the bare repo's HEAD tree has all 4 files.
        let bare = Repository::open_bare(&bare_path).unwrap();
        let head_tree = bare.head().unwrap().peel_to_tree().unwrap();
        let blob_count = count_tree_blobs(&bare, &head_tree);
        assert_eq!(
            blob_count, 4,
            "HEAD tree must contain all 3 originals + 1 new file"
        );
    }

    /// Records an incremental update directly in the persistent repository.
    fn add_commit(work_path: &Path, file: &str, content: &[u8], msg: &str) {
        let bare_path = work_path.parent().unwrap().join("repo.git");
        let prepared = prepare_worktree(&bare_path, work_path).unwrap();
        scratch::write(work_path, Path::new(file), content).unwrap();
        assert!(commit_all(&prepared, msg).unwrap());
    }

    /// Collects all emitted commit versions into a vector.
    fn versions(bare_path: &Path, since: Option<&str>) -> Vec<CommitVersions> {
        let mut all = Vec::new();
        collect_versions(bare_path, since, |c| {
            all.push(c);
            Ok(())
        })
        .unwrap();
        all
    }

    /// Builds a minimal CSAF document with the given tracking fields.
    fn csaf(status: &str, version: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"document": {"tracking": {
            "status": status, "version": version, "current_release_date": "2024-01-01T00:00:00Z"
        }}}))
        .unwrap()
    }

    /// Records versions oldest-first with tracking fields and survives scratch cleanup.
    #[test]
    fn collect_versions_records_tracking_history() {
        let file = "example.com/doc.json";
        let (dir, bare_path) = create_test_repo(&[
            (file, &csaf("draft", "1")),
            ("metadata/provider-metadata.json", b"{}"),
            ("example.com/doc.json.sha256", b"digest"),
        ]);
        let work_path = dir.path().join("work");
        add_commit(&work_path, file, &csaf("interim", "2"), "second");
        add_commit(&work_path, "example.com/other.txt", b"ignored", "unrelated");
        add_commit(&work_path, file, &csaf("final", "3"), "third");
        fs::remove_dir_all(&work_path).unwrap();

        let all = versions(&bare_path, None);
        assert_eq!(
            all.iter().map(|c| c.message.as_str()).collect::<Vec<_>>(),
            ["initial", "second", "third"]
        );
        assert!(all.iter().all(|c| c.documents.len() == 1));
        let doc = &all[2].documents[0];
        assert_eq!(doc.url, "https://example.com/doc.json");
        assert_eq!(
            doc.tracking,
            TrackingSummary {
                status: Some("final".into()),
                version: Some("3".into()),
                current_release_date: Some("2024-01-01T00:00:00Z".into()),
            }
        );
        let original = read_blob(&bare_path, &all[0].documents[0].blob_id)
            .unwrap()
            .unwrap();
        assert_eq!(original, csaf("draft", "1"));
    }

    /// Only commits after `since` are visited.
    #[test]
    fn collect_versions_since_checkpoint() {
        let file = "example.com/doc.json";
        let (dir, bare_path) = create_test_repo(&[(file, b"{}")]);
        let checkpoint = versions(&bare_path, None)[0].commit_id.clone();
        add_commit(&dir.path().join("work"), file, b"{\"v\":2}", "second");
        let all = versions(&bare_path, Some(&checkpoint));
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].message, "second");
    }

    /// Reverts and re-additions after deletion are distinct versions.
    #[test]
    fn collect_versions_records_reverts_and_readditions() {
        let file = "example.com/doc.json";
        let (dir, bare_path) = create_test_repo(&[(file, b"a")]);
        let work_path = dir.path().join("work");
        add_commit(&work_path, file, b"b", "b");
        add_commit(&work_path, file, b"a", "revert");
        let repo = Repository::open_bare(&bare_path).unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        let mut index = Index::new().unwrap();
        index.read_tree(&parent.tree().unwrap()).unwrap();
        index.remove_path(Path::new(file)).unwrap();
        let tree = repo.find_tree(index.write_tree_to(&repo).unwrap()).unwrap();
        let sig = Signature::now("test", "test@example.com").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "delete", &tree, &[&parent])
            .unwrap();
        add_commit(&work_path, file, b"a", "re-add");
        assert_eq!(
            versions(&bare_path, None)
                .iter()
                .map(|c| c.message.as_str())
                .collect::<Vec<_>>(),
            ["initial", "b", "revert", "re-add"]
        );
    }

    /// Tracking extraction tolerates invalid JSON and non-string scalars.
    #[test]
    fn parse_tracking_is_lenient() {
        assert_eq!(parse_tracking(b"not json"), TrackingSummary::default());
        assert_eq!(
            parse_tracking(br#"{"document":{"tracking":{"version":2,"status":["x"]}}}"#),
            TrackingSummary {
                version: Some("2".into()),
                ..Default::default()
            }
        );
    }

    /// Missing blobs are reported as absent.
    #[test]
    fn read_blob_returns_none_for_missing() {
        let (_dir, bare_path) = create_test_repo(&[("example.com/doc.json", b"{}")]);
        let missing = "0123456789012345678901234567890123456789";
        assert!(read_blob(&bare_path, missing).unwrap().is_none());
    }

    #[test]
    fn diff_documents_detects_changes() {
        let v1 = br#"{"document":{"title":"Advisory v1","category":"csaf_vex"}}"#;
        let v2 = br#"{"document":{"title":"Advisory v2","category":"csaf_vex"}}"#;
        let diff = diff_documents(v1, v2);
        assert!(diff.iter().any(|l| matches!(l.tag, DiffTag::Delete)));
        let insert_text: String = diff
            .iter()
            .filter(|l| matches!(l.tag, DiffTag::Insert))
            .map(|l| l.content.clone())
            .collect();
        assert!(insert_text.contains("Advisory v2"));
    }

    /// Formatting-only changes (different whitespace, same JSON semantics)
    /// should still produce a visible diff by falling back to raw content.
    #[test]
    fn diff_falls_back_to_raw_for_formatting_only_changes() {
        let compact = br#"{"document":{"title":"hello"}}"#;
        let pretty = b"{\n  \"document\": {\n    \"title\": \"hello\"\n  }\n}\n";
        let diff = diff_documents(compact, pretty);
        assert!(
            diff.iter()
                .any(|l| matches!(l.tag, DiffTag::Insert | DiffTag::Delete)),
            "formatting-only change should still produce a visible diff"
        );
    }
}

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    str::from_utf8,
    time::Instant,
};

use super::scratch;
use crate::models::result::{DiffLineInfo, DiffTag};
use anyhow::{Context, Result, ensure};
use git2::{BranchType, ErrorCode, Index, IndexEntry, IndexTime, Oid, Repository, Signature, Tree};
use serde::Serialize;
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
/// Incremental runs start empty; full runs materialize HEAD with compressed advisories.
/// The caller must hold the provider's pipeline lock until committing and cleanup.
pub fn prepare_worktree(
    repo_path: &Path,
    worktree_path: &Path,
    incremental: bool,
) -> Result<PreparedWorktree> {
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
        if !incremental {
            for entry in index.iter() {
                let relative = Path::new(from_utf8(&entry.path)?);
                let blob = repo.find_blob(entry.id)?;
                scratch::write(&worktree.worktree_path, relative, blob.content())
                    .with_context(|| format!("Failed to materialize {}", relative.display()))?;
            }
        }
    }
    index.write()?;
    tracing::info!(
        repository = %repo_path.display(),
        incremental,
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
pub fn commit_all(worktree: &PreparedWorktree, message: &str) -> Result<bool> {
    let worktree_path = &worktree.worktree_path;
    // Index::open would silently create an empty index if scratch data was lost.
    fs::metadata(worktree_path.join(".git/index")).context("Missing prepared worktree index")?;
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
            if logical != relative {
                ensure!(
                    !worktree_path.join(&logical).exists(),
                    "Both plain and compressed scratch files exist: {}",
                    logical.display()
                );
                let data = scratch::read(entry.path())?;
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
                index.add_path(relative)?;
            }
        }
    }
    index.write()?;

    let tree_oid = index.write_tree()?;
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

/// A version of a document as recorded in a git commit.
#[derive(Debug, Serialize)]
pub struct DocumentVersion {
    /// Commit SHA where this version was recorded.
    pub commit_id: String,
    /// Commit timestamp as Unix seconds.
    pub timestamp: i64,
    /// Commit message.
    pub message: String,
    /// Whether this is the most recent (HEAD) version.
    pub is_latest: bool,
}

/// Looks up a blob OID at a known path within a commit's tree.
fn blob_oid_at_path(tree: &Tree<'_>, path: &str) -> Result<Option<Oid>> {
    match tree.get_path(std::path::Path::new(path)) {
        Ok(entry) => Ok(Some(entry.id())),
        Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Collects blob OIDs for multiple paths in a single tree walk.
///
/// Given a set of git paths we're interested in, walks the tree once and
/// returns a map from path to OID. This is O(tree_size) regardless of how
/// many paths we're looking for, much faster than calling `blob_oid_at_path`
/// repeatedly for large flat directories.
fn collect_blob_oids(tree: &Tree<'_>, paths: &HashSet<String>) -> HashMap<String, Oid> {
    let mut result = HashMap::new();

    tree.walk(git2::TreeWalkMode::PreOrder, |dir, entry| {
        if entry.kind() == Some(git2::ObjectType::Blob) {
            let full_path = if dir.is_empty() {
                entry.name().unwrap_or("").to_string()
            } else {
                format!("{}{}", dir, entry.name().unwrap_or(""))
            };

            if paths.contains(&full_path) {
                result.insert(full_path, entry.id());
            }
        }
        git2::TreeWalkResult::Ok
    })
    .ok();

    result
}

/// Converts a document URL to its git tree path: `<domain>/<url_path>`.
///
/// The worktree (and thus the git tree) stores files as
/// `<domain>/<url_path>`, mirroring the URL structure directly.
fn url_to_git_path(url: &str) -> Result<Option<String>> {
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

/// Returns the commits where a document changed, newest first.
pub fn document_versions(
    repo_path: &Path,
    url: &str,
    max_entries: usize,
) -> Result<Option<Vec<DocumentVersion>>> {
    let repo = Repository::open_bare(repo_path)?;

    let Ok(head) = repo.head() else {
        return Ok(None);
    };
    let head_commit = head.peel_to_commit()?;
    let head_tree = head_commit.tree()?;

    let Some(file_path) = url_to_git_path(url)? else {
        return Ok(None);
    };
    if blob_oid_at_path(&head_tree, &file_path)?.is_none() {
        return Ok(None);
    }

    let mut revwalk = repo.revwalk()?;
    revwalk.push(head.target().context("HEAD has no target")?)?;

    let mut versions = Vec::new();
    let mut prev_blob_oid: Option<Oid> = None;
    let mut is_first = true;

    for oid in revwalk {
        let oid = oid?;
        let commit = repo.find_commit(oid)?;
        let tree = commit.tree()?;

        let current_oid = blob_oid_at_path(&tree, &file_path)?;

        let changed = match (current_oid, prev_blob_oid) {
            (Some(cur), Some(prev)) => cur != prev,
            (Some(_), None) => true,
            (None, Some(_)) => {
                prev_blob_oid = None;
                continue;
            }
            (None, None) => {
                continue;
            }
        };

        prev_blob_oid = current_oid;

        if changed {
            versions.push(DocumentVersion {
                commit_id: oid.to_string(),
                timestamp: commit.time().seconds(),
                message: commit.message().unwrap_or("").to_string(),
                is_latest: is_first,
            });
            if versions.len() >= max_entries {
                break;
            }
        }

        is_first = false;
    }

    Ok(Some(versions))
}

/// Counts the distinct versions for multiple documents in a single revwalk.
///
/// Returns a map from document URL to version count. Documents not found
/// in the repo are omitted. More efficient than calling `document_versions`
/// per document because the repo and revwalk are shared.
pub fn document_version_counts(repo_path: &Path, urls: &[&str]) -> Result<HashMap<String, u32>> {
    let repo = Repository::open_bare(repo_path)?;

    let Ok(head) = repo.head() else {
        return Ok(HashMap::new());
    };

    let paths: Vec<(String, String)> = urls
        .iter()
        .filter_map(|url| {
            url_to_git_path(url)
                .ok()
                .flatten()
                .map(|path| ((*url).to_string(), path))
        })
        .collect();

    if paths.is_empty() {
        return Ok(HashMap::new());
    }

    // Build a HashSet of git paths for efficient tree walking
    let git_paths: HashSet<String> = paths.iter().map(|(_, p)| p.clone()).collect();

    let mut revwalk = repo.revwalk()?;
    revwalk.push(head.target().context("HEAD has no target")?)?;

    let mut prev_oids: HashMap<&str, Option<Oid>> = HashMap::new();
    let mut counts: HashMap<&str, u32> = HashMap::new();

    for oid in revwalk {
        let oid = oid?;
        let commit = repo.find_commit(oid)?;
        let tree = commit.tree()?;

        // Walk tree once per commit, collect all blob OIDs we care about
        let current_oids = collect_blob_oids(&tree, &git_paths);

        for (url, git_path) in &paths {
            let current_oid = current_oids.get(git_path).copied();
            let prev = prev_oids.get(url.as_str()).copied().flatten();

            let changed = match (current_oid, prev) {
                (Some(cur), Some(prv)) => cur != prv,
                (Some(_), None) => true,
                (None, Some(_)) => {
                    prev_oids.insert(url, None);
                    continue;
                }
                (None, None) => continue,
            };

            prev_oids.insert(url, current_oid);

            if changed {
                *counts.entry(url).or_default() += 1;
            }
        }
    }

    Ok(counts
        .into_iter()
        .map(|(url, count)| (url.to_string(), count))
        .collect())
}

/// Reads the raw content of a document blob at a specific commit.
pub fn read_document_blob(
    repo_path: &Path,
    url: &str,
    commit_id: &str,
) -> Result<Option<(Vec<u8>, i64)>> {
    let repo = Repository::open_bare(repo_path)?;
    let oid = Oid::from_str(commit_id).context("invalid commit ID")?;
    let commit = repo.find_commit(oid)?;
    let tree = commit.tree()?;

    let Some(file_path) = url_to_git_path(url)? else {
        return Ok(None);
    };
    let Some(blob_oid) = blob_oid_at_path(&tree, &file_path)? else {
        return Ok(None);
    };

    let blob = repo.find_blob(blob_oid)?;
    Ok(Some((blob.content().to_vec(), commit.time().seconds())))
}

/// Computes a structured line diff between two versions of a document.
///
/// Both blobs are pretty-printed as JSON to normalize formatting. If the
/// pretty-printed content is identical (formatting-only change), falls back
/// to diffing the raw content so the actual differences remain visible.
pub fn diff_document_versions(
    repo_path: &Path,
    url: &str,
    old_commit_id: &str,
    new_commit_id: &str,
) -> Result<Option<Vec<DiffLineInfo>>> {
    let old = read_document_blob(repo_path, url, old_commit_id)?;
    let new = read_document_blob(repo_path, url, new_commit_id)?;

    let (Some((old_blob, _)), Some((new_blob, _))) = (old, new) else {
        return Ok(None);
    };

    let old_pretty = pretty_print_or_raw(&old_blob);
    let new_pretty = pretty_print_or_raw(&new_blob);

    let (old_text, new_text) = if old_pretty == new_pretty {
        let old_raw = String::from_utf8_lossy(&old_blob).into_owned();
        let new_raw = String::from_utf8_lossy(&new_blob).into_owned();
        (old_raw, new_raw)
    } else {
        (old_pretty, new_pretty)
    };

    let diff = similar::TextDiff::from_lines(&old_text, &new_text);

    let lines = diff
        .iter_all_changes()
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
        .collect();

    Ok(Some(lines))
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
    use std::{fs, path::PathBuf};

    /// Creates provider history through the production setup and commit path.
    fn create_test_repo(files: &[(&str, &[u8])]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let bare_path = dir.path().join("repo.git");
        let work_path = dir.path().join("work");
        let prepared = prepare_worktree(&bare_path, &work_path, false).unwrap();
        for (path, content) in files {
            let full = work_path.join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(&full, content).unwrap();
        }
        assert!(commit_all(&prepared, "initial").unwrap());
        (dir, bare_path)
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
        let prepared = prepare_worktree(&bare_path, &work_path, true).unwrap();
        assert!(!work_path.join("example.com").exists());
        assert!(!work_path.join(".git/objects").exists());
        assert_eq!(fs::read_dir(work_path.join(".git")).unwrap().count(), 1);
        assert_eq!(Index::open(&work_path.join(".git/index")).unwrap().len(), 1);
        assert!(!commit_all(&prepared, "no changes").unwrap());
        assert_eq!(fs::read(bare_path.join("config")).unwrap(), config);
        assert_eq!(fs::read(bare_path.join("HEAD")).unwrap(), head);
        assert!(!bare_path.join("index").exists());
        assert!(Repository::open_bare(&bare_path).unwrap().is_bare());
        assert_eq!(
            fs::read_dir(bare_path.join("objects/pack"))
                .unwrap()
                .count(),
            0
        );
        fs::remove_dir_all(&work_path).unwrap();
        assert_eq!(
            read_head_blob(&bare_path, "example.com/doc.json")
                .unwrap()
                .unwrap(),
            b"old"
        );
    }

    /// Full syncs compress existing history and commit original bytes without extra versions.
    #[test]
    fn full_worktree_checks_out_current_tree() {
        let (dir, bare_path) = create_test_repo(&[("example.com/doc.json", b"old")]);
        let work_path = dir.path().join("full");
        let prepared = prepare_worktree(&bare_path, &work_path, false).unwrap();
        assert_eq!(
            scratch::read(&work_path.join("example.com/doc.json.zst")).unwrap(),
            b"old"
        );
        assert!(!commit_all(&prepared, "unchanged").unwrap());
        assert!(!work_path.join("example.com/doc.json").exists());
        scratch::write(&work_path, Path::new("example.com/doc.json"), b"new").unwrap();
        assert!(commit_all(&prepared, "changed").unwrap());
        assert_eq!(
            read_head_blob(&bare_path, "example.com/doc.json")
                .unwrap()
                .unwrap(),
            b"new"
        );
    }

    /// Direct commits preserve versions, counts, and diffs after deleting scratch files.
    #[test]
    fn direct_commit_history_survives_cleanup() {
        let (dir, bare_path) = create_test_repo(&[("example.com/doc.json", br#"{"v":1}"#)]);
        let work_path = dir.path().join("work");
        add_commit(&work_path, "example.com/doc.json", br#"{"v":2}"#, "second");
        add_commit(&work_path, "example.com/doc.json", br#"{"v":3}"#, "third");
        fs::remove_dir_all(&work_path).unwrap();
        let url = "https://example.com/doc.json";
        let versions = document_versions(&bare_path, url, 50).unwrap().unwrap();
        assert_eq!(versions.len(), 3);
        assert_eq!(document_version_counts(&bare_path, &[url]).unwrap()[url], 3);
        let (original, _) = read_document_blob(&bare_path, url, &versions[2].commit_id)
            .unwrap()
            .unwrap();
        assert_eq!(original, br#"{"v":1}"#);
        let diff = diff_document_versions(
            &bare_path,
            url,
            &versions[2].commit_id,
            &versions[0].commit_id,
        )
        .unwrap()
        .unwrap();
        assert!(diff.iter().any(|line| matches!(line.tag, DiffTag::Insert)));
        assert!(diff.iter().any(|line| matches!(line.tag, DiffTag::Delete)));
    }

    /// Empty repositories retain their configured branch name for the first commit.
    #[test]
    fn initial_sync_supports_custom_branch() {
        let dir = tempfile::tempdir().unwrap();
        let bare_path = dir.path().join("repo.git");
        let bare = init_bare(&bare_path).unwrap();
        bare.set_head("refs/heads/provider/history").unwrap();
        let work_path = dir.path().join("work");
        let prepared = prepare_worktree(&bare_path, &work_path, true).unwrap();
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
        let prepared = prepare_worktree(&bare_path, &dir.path().join("work"), true).unwrap();
        assert_eq!(prepared.branch, original);
        assert_eq!(prepared.base_commit, Some(oid));
        assert_eq!(bare.head().unwrap().target(), Some(oid));
    }

    /// Corrupt objects must fail setup rather than start a new history.
    #[test]
    fn worktree_propagates_missing_commit_object() {
        let (dir, bare_path) = create_test_repo(&[("doc.json", b"old")]);
        let bare = Repository::open_bare(&bare_path).unwrap();
        let oid = bare.head().unwrap().target().unwrap().to_string();
        drop(bare);
        fs::remove_file(bare_path.join("objects").join(&oid[..2]).join(&oid[2..])).unwrap();
        assert!(prepare_worktree(&bare_path, &dir.path().join("work"), true).is_err());
    }

    /// Competing updates are rejected for both existing and initially absent branches.
    #[test]
    fn direct_commit_rejects_branch_changes() {
        for initial in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let bare_path = dir.path().join("repo.git");
            let work_path = dir.path().join("work");
            let mut prepared = prepare_worktree(&bare_path, &work_path, true).unwrap();
            if !initial {
                fs::write(work_path.join("doc.json"), b"old").unwrap();
                commit_all(&prepared, "initial").unwrap();
                prepared = prepare_worktree(&bare_path, &work_path, true).unwrap();
            }
            let competing_path = dir.path().join("competing");
            let competing = prepare_worktree(&bare_path, &competing_path, true).unwrap();
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

    /// A lost index cannot silently discard the provider's existing documents.
    #[test]
    fn direct_commit_rejects_missing_index() {
        let (dir, bare_path) = create_test_repo(&[("doc.json", b"old")]);
        let work_path = dir.path().join("work");
        let prepared = prepare_worktree(&bare_path, &work_path, true).unwrap();
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
                let prepared = prepare_worktree(&repo_path, &work_path, true).unwrap();
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

    #[test]
    fn document_versions_finds_by_url_path() {
        let file_path =
            "security.access.redhat.com/data/csaf/v2/advisories/2024/rhsa-2024_1234.json";
        let (_dir, bare_path) = create_test_repo(&[(file_path, b"{}")]);

        let url =
            "https://security.access.redhat.com/data/csaf/v2/advisories/2024/rhsa-2024_1234.json";
        let versions = document_versions(&bare_path, url, 50).unwrap();
        assert!(versions.is_some(), "should find document by URL");
        assert_eq!(versions.unwrap().len(), 1);
    }

    #[test]
    fn document_versions_returns_none_for_missing() {
        let file_path =
            "security.access.redhat.com/data/csaf/v2/advisories/2024/rhsa-2024_1234.json";
        let (_dir, bare_path) = create_test_repo(&[(file_path, b"{}")]);

        let url = "https://example.com/nonexistent.json";
        let versions = document_versions(&bare_path, url, 50).unwrap();
        assert!(
            versions.is_none(),
            "should return None for missing document"
        );
    }

    #[test]
    fn read_document_blob_finds_by_url_path() {
        let file_path =
            "security.access.redhat.com/data/csaf/v2/advisories/2024/rhsa-2024_1234.json";
        let content = b"{\"doc\": true}";
        let (_dir, bare_path) = create_test_repo(&[(file_path, content)]);

        let repo = Repository::open_bare(&bare_path).unwrap();
        let head = repo.head().unwrap();
        let commit_id = head.target().unwrap().to_string();

        let url =
            "https://security.access.redhat.com/data/csaf/v2/advisories/2024/rhsa-2024_1234.json";
        let result = read_document_blob(&bare_path, url, &commit_id).unwrap();
        assert!(result.is_some(), "should find blob by URL");
        let (blob, _ts) = result.unwrap();
        assert_eq!(blob, content);
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
        let prepared = prepare_worktree(&bare_path, &inc_work, true).unwrap();

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
        let prepared = prepare_worktree(&bare_path, work_path, true).unwrap();
        scratch::write(work_path, Path::new(file), content).unwrap();
        assert!(commit_all(&prepared, msg).unwrap());
    }

    #[test]
    fn diff_document_versions_detects_changes() {
        let file_path =
            "security.access.redhat.com/data/csaf/v2/advisories/2024/rhsa-2024_1234.json";
        let v1 = br#"{"document":{"title":"Advisory v1","category":"csaf_vex"}}"#;
        let (dir, bare_path) = create_test_repo(&[(file_path, v1)]);
        let work_path = dir.path().join("work");

        let v2 = br#"{"document":{"title":"Advisory v2","category":"csaf_vex"}}"#;
        add_commit(&work_path, file_path, v2, "v2");

        let url =
            "https://security.access.redhat.com/data/csaf/v2/advisories/2024/rhsa-2024_1234.json";
        let versions = document_versions(&bare_path, url, 50).unwrap().unwrap();
        assert_eq!(versions.len(), 2);

        let old_id = &versions[1].commit_id;
        let new_id = &versions[0].commit_id;

        let diff = diff_document_versions(&bare_path, url, old_id, new_id)
            .unwrap()
            .unwrap();

        let has_insert = diff.iter().any(|l| matches!(l.tag, DiffTag::Insert));
        let has_delete = diff.iter().any(|l| matches!(l.tag, DiffTag::Delete));
        assert!(has_insert, "diff should contain inserted lines");
        assert!(has_delete, "diff should contain deleted lines");

        let insert_text: String = diff
            .iter()
            .filter(|l| matches!(l.tag, DiffTag::Insert))
            .map(|l| l.content.clone())
            .collect();
        assert!(
            insert_text.contains("Advisory v2"),
            "inserted text should contain new title"
        );
    }

    #[test]
    fn errata_url_does_not_find_document() {
        let file_path =
            "security.access.redhat.com/data/csaf/v2/advisories/2024/rhsa-2024_1234.json";
        let (_dir, bare_path) = create_test_repo(&[(file_path, b"{}")]);

        let errata_url = "https://access.redhat.com/errata/RHSA-2024:1234";
        let versions = document_versions(&bare_path, errata_url, 50).unwrap();
        assert!(
            versions.is_none(),
            "errata URL should NOT find a document stored by distribution URL"
        );
    }

    /// Tests the server-side diff resolution: given a single commit ID,
    /// find the next newer version and compute the diff.
    #[test]
    fn diff_resolution_by_single_commit_id() {
        let file_path =
            "security.access.redhat.com/data/csaf/v2/advisories/2024/rhsa-2024_5678.json";
        let v1 = br#"{"document":{"title":"v1"}}"#;
        let (dir, bare_path) = create_test_repo(&[(file_path, v1)]);
        let work_path = dir.path().join("work");

        let v2 = br#"{"document":{"title":"v2"}}"#;
        add_commit(&work_path, file_path, v2, "v2");

        let v3 = br#"{"document":{"title":"v3"}}"#;
        add_commit(&work_path, file_path, v3, "v3");

        let url =
            "https://security.access.redhat.com/data/csaf/v2/advisories/2024/rhsa-2024_5678.json";
        let versions = document_versions(&bare_path, url, 50).unwrap().unwrap();
        assert_eq!(versions.len(), 3);
        assert!(versions[0].is_latest, "first version should be latest");
        assert!(!versions[1].is_latest);
        assert!(!versions[2].is_latest);

        // Simulate Storage::diff_document_versions: pass only the selected
        // commit_id (v2, the middle version) and resolve the newer version.
        let selected_commit = &versions[1].commit_id;
        let pos = versions
            .iter()
            .position(|v| v.commit_id == *selected_commit)
            .expect("commit should be in versions list");
        assert_ne!(pos, 0, "should not be the latest");
        let new_commit = &versions[pos - 1].commit_id;
        let diff = diff_document_versions(&bare_path, url, selected_commit, new_commit)
            .unwrap()
            .expect("diff should be available");
        assert!(
            diff.iter().any(|l| matches!(l.tag, DiffTag::Insert)),
            "diff should have insertions"
        );

        // Latest version (pos == 0) should have no diff.
        let latest_commit = &versions[0].commit_id;
        let latest_pos = versions
            .iter()
            .position(|v| v.commit_id == *latest_commit)
            .unwrap();
        assert_eq!(latest_pos, 0);
    }

    /// Formatting-only changes (different whitespace, same JSON semantics)
    /// should still produce a visible diff by falling back to raw content.
    #[test]
    fn diff_falls_back_to_raw_for_formatting_only_changes() {
        let file_path = "example.com/advisories/2024/fmt.json";
        let compact = br#"{"document":{"title":"hello"}}"#;
        let (dir, bare_path) = create_test_repo(&[(file_path, compact)]);
        let work_path = dir.path().join("work");

        let pretty = b"{\n  \"document\": {\n    \"title\": \"hello\"\n  }\n}\n";
        add_commit(&work_path, file_path, pretty, "pretty");

        let url = "https://example.com/advisories/2024/fmt.json";
        let versions = document_versions(&bare_path, url, 50).unwrap().unwrap();
        assert_eq!(versions.len(), 2, "both commits should appear as versions");

        let diff = diff_document_versions(
            &bare_path,
            url,
            &versions[1].commit_id,
            &versions[0].commit_id,
        )
        .unwrap()
        .expect("diff should be available");

        let has_changes = diff
            .iter()
            .any(|l| matches!(l.tag, DiffTag::Insert | DiffTag::Delete));
        assert!(
            has_changes,
            "formatting-only change should still produce a visible diff"
        );
    }

    #[test]
    fn version_counts_batch() {
        let file_a = "example.com/advisories/2024/adv-001.json";
        let file_b = "example.com/advisories/2024/adv-002.json";
        let (_dir, bare_path) = create_test_repo(&[(file_a, b"{\"v\":1}"), (file_b, b"{\"v\":1}")]);

        let work_path = _dir.path().join("work");
        add_commit(&work_path, file_a, b"{\"v\":2}", "update a");

        let url_a = "https://example.com/advisories/2024/adv-001.json";
        let url_b = "https://example.com/advisories/2024/adv-002.json";

        let counts = document_version_counts(&bare_path, &[url_a, url_b]).unwrap();
        assert_eq!(
            counts.get(url_a).copied(),
            Some(2),
            "modified file should have 2 versions"
        );
        assert_eq!(
            counts.get(url_b).copied(),
            Some(1),
            "unmodified file should have 1 version"
        );
    }
}

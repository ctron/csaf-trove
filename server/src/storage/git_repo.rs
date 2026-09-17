use std::path::Path;

use anyhow::{Context, Result};
use git2::{Oid, Repository, Signature, Tree};
use serde::Serialize;
use walkdir::WalkDir;

use crate::models::result::{DiffLineInfo, DiffTag};

/// Opens an existing bare repo or initializes a new one.
pub fn init_bare(path: &Path) -> Result<Repository> {
    if path.exists() {
        Repository::open_bare(path).context("Failed to open bare repo")
    } else {
        Repository::init_bare(path).context("Failed to init bare repo")
    }
}

/// Stages working-directory files into the index and commits, then pushes to the bare repo.
///
/// Uses `add_path` per file instead of `add_all` so that existing index entries
/// (e.g. from a previous `read_tree` in incremental mode) are preserved for files
/// not present on disk.  Returns `false` if nothing changed.
pub fn commit_all(repo_path: &Path, worktree_path: &Path, message: &str) -> Result<bool> {
    let repo = Repository::open(worktree_path)?;
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
            index.add_path(relative)?;
        }
    }
    index.write()?;

    let tree_oid = index.write_tree()?;
    let tree = repo.find_tree(tree_oid)?;

    let sig = Signature::now("csaf-trove", "csaf-trove@localhost")?;

    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());

    if let Some(ref parent) = parent
        && parent.tree()?.id() == tree_oid
    {
        tracing::debug!("No changes to commit for {}", repo_path.display());
        return Ok(false);
    }

    let parents: Vec<&git2::Commit> = parent.as_ref().map(|p| vec![p]).unwrap_or_default();
    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &parents)?;

    push_to_bare(&repo)?;

    Ok(true)
}

/// Pushes the worktree commit back to the bare repo via its `origin` remote.
///
/// After pushing, ensures the bare repo's HEAD points to the pushed branch
/// so that subsequent clones see the full history.
fn push_to_bare(worktree_repo: &Repository) -> Result<()> {
    let head = worktree_repo
        .head()
        .context("worktree has no HEAD after commit")?;
    let branch = head
        .shorthand()
        .context("HEAD branch name is not valid UTF-8")?;
    let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");

    let mut remote = worktree_repo
        .find_remote("origin")
        .context("worktree has no origin remote")?;
    remote
        .push(&[&refspec], None)
        .context("failed to push worktree commit to bare repo")?;

    if let Ok(url) = remote.url().map(String::from) {
        let bare_path = std::path::Path::new(&url);
        if bare_path.exists()
            && let Ok(bare) = Repository::open_bare(bare_path)
        {
            let target_ref = format!("refs/heads/{branch}");
            if bare.head().is_err() {
                bare.set_head(&target_ref).ok();
            }
        }
    }

    Ok(())
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
/// Both blobs are pretty-printed as JSON to normalize formatting. If JSON
/// parsing fails for either blob, the raw UTF-8 content is used instead.
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

    let old_text = pretty_print_or_raw(&old_blob);
    let new_text = pretty_print_or_raw(&new_blob);

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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    /// Creates a bare repo with a worktree, commits files, and pushes.
    fn create_test_repo(files: &[(&str, &[u8])]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let bare_path = dir.path().join("repo.git");
        let work_path = dir.path().join("work");

        Repository::init_bare(&bare_path).unwrap();
        let repo = Repository::init(&work_path).unwrap();
        repo.remote("origin", bare_path.to_str().unwrap()).unwrap();

        for (path, content) in files {
            let full = work_path.join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(&full, content).unwrap();
        }

        let mut index = repo.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let sig = Signature::now("test", "test@test").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "initial", &tree, &[])
            .unwrap();

        push_to_bare(&repo).unwrap();

        (dir, bare_path)
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

        // Set up an incremental-style worktree: index from HEAD, no files on disk.
        let inc_work = _dir.path().join("incremental");
        let repo = Repository::init(&inc_work).unwrap();
        repo.remote("origin", bare_path.to_str().unwrap()).unwrap();
        let mut remote = repo.find_remote("origin").unwrap();
        remote
            .fetch(&["refs/heads/*:refs/remotes/origin/*"], None, None)
            .unwrap();

        let origin_ref = repo.find_reference("refs/remotes/origin/master").unwrap();
        let origin_commit = origin_ref.peel_to_commit().unwrap();
        let mut index = repo.index().unwrap();
        index.read_tree(&origin_commit.tree().unwrap()).unwrap();
        index.write().unwrap();
        repo.branch("master", &origin_commit, false).unwrap();
        repo.set_head("refs/heads/master").unwrap();

        // Add one new file to the working directory (simulating incremental sync).
        let new_file = inc_work.join("example.com/advisories/2025/adv-004.json");
        fs::create_dir_all(new_file.parent().unwrap()).unwrap();
        fs::write(&new_file, b"{\"a\":4}").unwrap();

        // commit_all must preserve the 3 existing index entries.
        let changed = commit_all(&bare_path, &inc_work, "incremental").unwrap();
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

    /// Creates a second commit by modifying a file in the existing worktree.
    fn add_commit(work_path: &Path, file: &str, content: &[u8], msg: &str) {
        let full = work_path.join(file);
        fs::write(&full, content).unwrap();

        let repo = Repository::open(work_path).unwrap();
        let mut index = repo.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let sig = Signature::now("test", "test@test").unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &[&parent])
            .unwrap();

        push_to_bare(&repo).unwrap();
    }

    #[test]
    fn diff_document_versions_detects_changes() {
        let file_path =
            "security.access.redhat.com/data/csaf/v2/advisories/2024/rhsa-2024_1234.json";
        let v1 = br#"{"document":{"title":"Advisory v1","category":"csaf_vex"}}"#;
        let dir = tempfile::tempdir().unwrap();
        let bare_path = dir.path().join("repo.git");
        let work_path = dir.path().join("work");

        Repository::init_bare(&bare_path).unwrap();
        let repo = Repository::init(&work_path).unwrap();
        repo.remote("origin", bare_path.to_str().unwrap()).unwrap();

        let full = work_path.join(file_path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(&full, v1).unwrap();

        let mut index = repo.index().unwrap();
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .unwrap();
        index.write().unwrap();
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let sig = Signature::now("test", "test@test").unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "v1", &tree, &[])
            .unwrap();
        push_to_bare(&repo).unwrap();

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
}

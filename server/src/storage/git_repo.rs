use std::path::Path;

use anyhow::{Context, Result};
use git2::{Oid, Repository, Signature, Tree};
use serde::Serialize;

/// Opens an existing bare repo or initializes a new one.
pub fn init_bare(path: &Path) -> Result<Repository> {
    if path.exists() {
        Repository::open_bare(path).context("Failed to open bare repo")
    } else {
        Repository::init_bare(path).context("Failed to init bare repo")
    }
}

/// Stages all changes in the worktree and commits, then pushes back to the bare repo.
///
/// Returns `false` if nothing changed.
pub fn commit_all(repo_path: &Path, worktree_path: &Path, message: &str) -> Result<bool> {
    let repo = Repository::open(worktree_path)?;
    let mut index = repo.index()?;

    index.add_all(["*"], git2::IndexAddOption::DEFAULT, None)?;
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("csaf-trove-test-{}", std::process::id()))
                .join(format!(
                    "{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Creates a bare repo with a worktree, commits files, and pushes.
    fn create_test_repo(files: &[(&str, &[u8])]) -> (TestDir, PathBuf) {
        let dir = TestDir::new();
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

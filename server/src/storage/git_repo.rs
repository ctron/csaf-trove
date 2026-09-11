use std::path::Path;

use anyhow::{Context, Result};
use git2::{Oid, Repository, Signature, Tree};
use serde::Serialize;

/// Summary of a single git commit.
#[derive(Debug, Serialize)]
pub struct CommitInfo {
    /// Full commit SHA.
    pub id: String,
    /// Commit message.
    pub message: String,
    /// Commit timestamp as Unix seconds.
    pub timestamp: i64,
    /// Number of files changed in this commit.
    pub files_changed: usize,
}

/// Opens an existing bare repo or initializes a new one.
pub fn init_bare(path: &Path) -> Result<Repository> {
    if path.exists() {
        Repository::open_bare(path).context("Failed to open bare repo")
    } else {
        Repository::init_bare(path).context("Failed to init bare repo")
    }
}

/// Returns the most recent commits from a bare repo.
pub fn log(repo_path: &Path, max_entries: usize) -> Result<Vec<CommitInfo>> {
    let repo = Repository::open_bare(repo_path)?;

    let Ok(head) = repo.head() else {
        return Ok(Vec::new());
    };

    let mut revwalk = repo.revwalk()?;
    revwalk.push(head.target().context("HEAD has no target")?)?;

    let mut entries = Vec::new();
    for oid in revwalk.take(max_entries) {
        let oid = oid?;
        let commit = repo.find_commit(oid)?;

        let files_changed = if let Some(parent) = commit.parents().next() {
            let diff =
                repo.diff_tree_to_tree(Some(&parent.tree()?), Some(&commit.tree()?), None)?;
            diff.stats()?.files_changed()
        } else {
            let diff = repo.diff_tree_to_tree(None, Some(&commit.tree()?), None)?;
            diff.stats()?.files_changed()
        };

        entries.push(CommitInfo {
            id: oid.to_string(),
            message: commit.message().unwrap_or("").to_string(),
            timestamp: commit.time().seconds(),
            files_changed,
        });
    }

    Ok(entries)
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

/// Recursively searches a git tree for a blob named `filename`.
///
/// Returns the full path and blob OID of the first match.
fn find_file_in_tree(
    repo: &Repository,
    tree: &Tree<'_>,
    filename: &str,
    prefix: &str,
) -> Result<Option<(String, Oid)>> {
    for entry in tree.iter() {
        let name = entry.name().context("non-UTF8 tree entry name")?;
        match entry.kind() {
            Some(git2::ObjectType::Blob) if name == filename => {
                let path = if prefix.is_empty() {
                    name.to_string()
                } else {
                    format!("{prefix}/{name}")
                };
                return Ok(Some((path, entry.id())));
            }
            Some(git2::ObjectType::Tree) => {
                let subtree = repo.find_tree(entry.id())?;
                let sub_prefix = if prefix.is_empty() {
                    name.to_string()
                } else {
                    format!("{prefix}/{name}")
                };
                if let Some(found) = find_file_in_tree(repo, &subtree, filename, &sub_prefix)? {
                    return Ok(Some(found));
                }
            }
            _ => {}
        }
    }
    Ok(None)
}

/// Looks up a blob OID at a known path within a commit's tree.
fn blob_oid_at_path(tree: &Tree<'_>, path: &str) -> Result<Option<Oid>> {
    match tree.get_path(std::path::Path::new(path)) {
        Ok(entry) => Ok(Some(entry.id())),
        Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Returns the commits where a document changed, newest first.
pub fn document_versions(
    repo_path: &Path,
    tracking_id: &str,
    max_entries: usize,
) -> Result<Option<Vec<DocumentVersion>>> {
    let repo = Repository::open_bare(repo_path)?;

    let Ok(head) = repo.head() else {
        return Ok(None);
    };
    let head_commit = head.peel_to_commit()?;
    let head_tree = head_commit.tree()?;

    let filename = format!("{tracking_id}.json");
    let Some((file_path, _)) = find_file_in_tree(&repo, &head_tree, &filename, "")? else {
        return Ok(None);
    };

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
    tracking_id: &str,
    commit_id: &str,
) -> Result<Option<(Vec<u8>, i64)>> {
    let repo = Repository::open_bare(repo_path)?;
    let oid = Oid::from_str(commit_id).context("invalid commit ID")?;
    let commit = repo.find_commit(oid)?;
    let tree = commit.tree()?;

    let filename = format!("{tracking_id}.json");
    let Some((_, blob_oid)) = find_file_in_tree(&repo, &tree, &filename, "")? else {
        return Ok(None);
    };

    let blob = repo.find_blob(blob_oid)?;
    Ok(Some((blob.content().to_vec(), commit.time().seconds())))
}

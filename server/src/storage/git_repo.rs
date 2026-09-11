use std::path::Path;

use anyhow::{Context, Result};
use git2::{Repository, Signature};
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
    Ok(())
}

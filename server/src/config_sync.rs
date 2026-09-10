use anyhow::{Context, Result};
use git2::{Repository, ResetType};
use std::{fs, path::Path};

/// Syncs source configuration files from the remote GitHub repository.
///
/// Clones the repository on first run, then fetches updates on subsequent runs.
/// Copies all `.toml` files from the repo's `sources/` directory into
/// `data_dir/sources/`.
pub fn sync(data_dir: &Path, repo_url: &str) -> Result<()> {
    let config_repo_dir = data_dir.join("config-repo");
    let sources_dir = data_dir.join("sources");

    if config_repo_dir.exists() {
        let repo = Repository::open(&config_repo_dir).context("Failed to open config repo")?;
        update_repo(&repo)?;
    } else {
        tracing::info!("Cloning config repo from {repo_url}");
        Repository::clone(repo_url, &config_repo_dir).context("Failed to clone config repo")?;
    }

    copy_sources(&config_repo_dir.join("sources"), &sources_dir)?;
    Ok(())
}

/// Fetches from origin and resets the working tree to the remote tracking branch.
fn update_repo(repo: &Repository) -> Result<()> {
    let head = repo.head().context("Failed to get HEAD")?;
    let branch_name = head.shorthand().unwrap_or("main");
    let remote_ref = format!("refs/remotes/origin/{branch_name}");

    let mut remote = repo
        .find_remote("origin")
        .context("Failed to find origin remote")?;
    remote
        .fetch(&[] as &[&str], None, None)
        .context("Failed to fetch from origin")?;
    drop(remote);

    let reference = repo
        .find_reference(&remote_ref)
        .context("Failed to find remote tracking branch")?;
    let commit = reference
        .peel_to_commit()
        .context("Failed to peel to commit")?;
    repo.reset(commit.as_object(), ResetType::Hard, None)
        .context("Failed to reset to remote HEAD")?;

    Ok(())
}

/// Replaces all `.toml` files in `dest` with those from `src`.
fn copy_sources(src: &Path, dest: &Path) -> Result<()> {
    if !src.exists() {
        tracing::warn!("No sources directory found in config repo");
        return Ok(());
    }

    if dest.exists() {
        for entry in fs::read_dir(dest)? {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "toml") {
                fs::remove_file(&path)?;
            }
        }
    }

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "toml")
            && let Some(file_name) = path.file_name()
        {
            fs::copy(&path, dest.join(file_name))?;
        }
    }

    Ok(())
}

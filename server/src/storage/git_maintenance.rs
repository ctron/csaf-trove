//! Consolidates Git object storage with the Git CLI, which libgit2 cannot do.

use crate::memory;
use anyhow::{Context, Result, ensure};
use std::{fs, path::Path, process::Command, time::Instant};

/// Prefix of temporary packs written by libgit2, left behind when a commit is interrupted.
const LIBGIT2_TEMP_PACK_PREFIX: &str = "pack_git2_";

/// Removes abandoned libgit2 pack files, then lets `git gc --auto` consolidate the repository.
///
/// Git only acts once its own thresholds are exceeded (too many packs or loose objects), so
/// healthy repositories are left untouched. The caller must hold the provider pipeline lock:
/// no commit may write to the repository concurrently.
pub fn maintain_repository(repo: &Path) -> Result<()> {
    if !repo.exists() {
        return Ok(());
    }
    let started = Instant::now();
    let pack_dir = repo.join("objects/pack");
    let removed = remove_temp_packs(&pack_dir)?;
    let packs_before = count_packs(&pack_dir)?;

    // Run in the foreground, so the pipeline lock covers it, and bound packing memory.
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "-c",
            "gc.autoDetach=false",
            "-c",
            "pack.threads=1",
            "-c",
            "pack.windowMemory=256m",
            "-c",
            "pack.deltaCacheSize=64m",
            "gc",
            "--auto",
            "--quiet",
        ])
        .output()
        .context("Failed to run git gc")?;
    ensure!(
        output.status.success(),
        "git gc failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );

    let memory = memory::usage();
    tracing::info!(
        repository = %repo.display(),
        removed_temp_packs = removed,
        packs_before,
        packs_after = count_packs(&pack_dir)?,
        elapsed_ms = started.elapsed().as_millis(),
        rss_mib = memory.map(|m| m.rss_mib),
        peak_mib = memory.map(|m| m.peak_mib),
        "Maintained Git repository"
    );
    Ok(())
}

/// Deletes temporary libgit2 pack files and returns how many were removed.
fn remove_temp_packs(pack_dir: &Path) -> Result<usize> {
    let mut removed = 0;
    for entry in read_pack_dir(pack_dir)? {
        let entry = entry?;
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(LIBGIT2_TEMP_PACK_PREFIX)
        {
            fs::remove_file(entry.path())
                .with_context(|| format!("Failed to remove {}", entry.path().display()))?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Counts the packfiles in a pack directory.
fn count_packs(pack_dir: &Path) -> Result<usize> {
    let mut count = 0;
    for entry in read_pack_dir(pack_dir)? {
        if entry?.path().extension().is_some_and(|ext| ext == "pack") {
            count += 1;
        }
    }
    Ok(count)
}

/// Lists a pack directory, treating a missing directory as empty.
fn read_pack_dir(pack_dir: &Path) -> Result<Vec<std::io::Result<fs::DirEntry>>> {
    match fs::read_dir(pack_dir) {
        Ok(entries) => Ok(entries.collect()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(error) => Err(error).with_context(|| format!("Failed to list {}", pack_dir.display())),
    }
}

#[cfg(test)]
mod tests;

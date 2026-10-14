use super::Storage;
use anyhow::Result;
use csaf_trove_common::overview::StorageUsage;
use std::{io::ErrorKind, path::Path};
use walkdir::WalkDir;

impl Storage {
    /// Measures persistent files without blocking the async request worker.
    pub async fn storage_usage(&self) -> Result<StorageUsage> {
        let paths = [
            self.repos_root.clone(),
            self.results_root.clone(),
            self.state_root.clone(),
            self.metrics_root.clone(),
        ];
        tokio::task::spawn_blocking(move || {
            Ok(StorageUsage {
                repositories: directory_size(&paths[0])?,
                results: directory_size(&paths[1])?,
                state: directory_size(&paths[2])?,
                metrics: directory_size(&paths[3])?,
            })
        })
        .await?
    }
}

/// Sums regular files without following symlinks, tolerating files removed during sync.
fn directory_size(path: &Path) -> Result<u64> {
    let mut bytes = 0;
    for entry in WalkDir::new(path) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error)
                if error
                    .io_error()
                    .is_some_and(|e| e.kind() == ErrorKind::NotFound) =>
            {
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if entry.file_type().is_file() {
            match entry.metadata() {
                Ok(metadata) => bytes += metadata.len(),
                Err(error)
                    if error
                        .io_error()
                        .is_some_and(|e| e.kind() == ErrorKind::NotFound) => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    /// Counts nested files in each category and excludes unrelated data.
    #[tokio::test]
    async fn counts_managed_storage() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let storage = Storage::new(dir.path())?;
        assert_eq!(storage.storage_usage().await?.total(), 0);
        fs::create_dir_all(storage.repos_root.join("provider.git/objects"))?;
        fs::write(
            storage.repos_root.join("provider.git/objects/pack"),
            [0; 17],
        )?;
        fs::write(storage.results_root.join("summary.json"), [0; 7])?;
        fs::write(storage.state_root.join("state.json"), [0; 3])?;
        fs::write(storage.metrics_root.join("metrics.json"), [0; 5])?;
        fs::write(dir.path().join("unrelated"), [0; 100])?;
        #[cfg(unix)]
        symlink(dir.path(), storage.repos_root.join("link"))?;
        let usage = storage.storage_usage().await?;
        assert_eq!(usage.repositories, 17);
        assert_eq!(usage.results, 7);
        assert_eq!(usage.state, 3);
        assert_eq!(usage.metrics, 5);
        assert_eq!(usage.total(), 32);
        Ok(())
    }
}

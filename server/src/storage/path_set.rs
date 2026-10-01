//! Async, disposable path deduplication with a bounded SQLite cache.

use anyhow::Result;
use sqlx::{
    Connection, QueryBuilder, Sqlite, SqliteConnection, query_scalar, raw_sql,
    sqlite::SqliteConnectOptions,
};
use std::path::Path;
use tempfile::{Builder, NamedTempFile};
use tokio::sync::Mutex;
#[cfg(test)]
use {super::git_processing::test_block_on, std::collections::BTreeSet};

/// Maximum paths returned by one cursor read.
pub const PATH_BATCH: usize = 256;

/// A sorted set beside provider data, never in a potentially memory-backed `/tmp`.
#[derive(Debug)]
pub struct PathSet {
    /// Single connection serializing access to the disposable table.
    db: Mutex<SqliteConnection>,
    /// Removes the database when the processing selection is dropped.
    _file: NamedTempFile,
    /// Number of unique paths.
    len: usize,
}

impl PathSet {
    /// Creates a bounded-cache database on the provider data volume.
    pub async fn new(directory: &Path) -> Result<Self> {
        let file = Builder::new()
            .prefix(".selection-")
            .tempfile_in(directory)?;
        let options = SqliteConnectOptions::new()
            .filename(file.path())
            .pragma("journal_mode", "OFF")
            .pragma("synchronous", "OFF")
            .pragma("cache_size", "-512")
            .pragma("mmap_size", "0")
            .pragma("temp_store", "FILE");
        let mut db = SqliteConnection::connect_with(&options).await?;
        raw_sql("CREATE TABLE paths (path TEXT PRIMARY KEY) WITHOUT ROWID; BEGIN;")
            .execute(&mut db)
            .await?;
        Ok(Self {
            db: Mutex::new(db),
            _file: file,
            len: 0,
        })
    }

    /// Inserts a bounded batch, ignoring duplicate paths.
    pub async fn insert_batch(&mut self, paths: &[String]) -> Result<()> {
        let db = self.db.get_mut();
        for paths in paths.chunks(PATH_BATCH) {
            let mut insert = QueryBuilder::<Sqlite>::new("INSERT OR IGNORE INTO paths (path) ");
            insert.push_values(paths, |mut row, path| {
                row.push_bind(path);
            });
            self.len += insert.build().execute(&mut *db).await?.rows_affected() as usize;
        }
        Ok(())
    }

    /// Adds a path without retaining its text in application memory.
    pub async fn insert(&mut self, path: String) -> Result<()> {
        self.insert_batch(&[path]).await
    }

    /// Returns the number of unique paths.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Reports whether no paths were inserted.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Reads a bounded page after an exclusive lexical cursor.
    pub async fn batch(&self, after: &str) -> Result<Vec<String>> {
        Ok(
            query_scalar("SELECT path FROM paths WHERE path > ? ORDER BY path LIMIT ?")
                .bind(after)
                .bind(PATH_BATCH as i64)
                .fetch_all(&mut *self.db.lock().await)
                .await?,
        )
    }

    /// Collects small test fixtures for assertions only.
    #[cfg(test)]
    pub fn values(&self) -> BTreeSet<String> {
        test_block_on(|| async {
            let mut values = BTreeSet::new();
            let mut cursor = String::new();
            loop {
                let batch = self.batch(&cursor).await.unwrap();
                if batch.is_empty() {
                    return values;
                }
                cursor = batch.last().unwrap().clone();
                values.extend(batch);
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Duplicate-heavy inputs spill to disk and remain ordered across bounded cursor pages.
    #[tokio::test]
    async fn spills_deduplicates_and_pages_without_retaining_paths() -> Result<()> {
        let directory = tempdir()?;
        let mut paths = PathSet::new(directory.path()).await?;
        let file = paths._file.path().to_owned();
        for start in (0..8192).step_by(PATH_BATCH) {
            let batch = (start..start + PATH_BATCH)
                .map(|index| format!("example.com/{:05}/{}.json", index % 4096, "x".repeat(256)))
                .collect::<Vec<_>>();
            paths.insert_batch(&batch).await?;
        }
        assert_eq!(paths.len(), 4096);
        assert!(
            file.metadata()?.len() > 512 * 1024,
            "selection must spill beyond its cache"
        );
        let mut cursor = String::new();
        let mut count = 0;
        loop {
            let batch = paths.batch(&cursor).await?;
            assert!(batch.len() <= PATH_BATCH);
            if batch.is_empty() {
                break;
            }
            for path in &batch {
                assert!(path > &cursor);
                cursor.clone_from(path);
                count += 1;
            }
        }
        assert_eq!(count, paths.len());
        drop(paths);
        assert!(!file.exists());
        Ok(())
    }
}

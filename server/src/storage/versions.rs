//! Per-document version records derived from the provider's git history.
mod test;

use super::{
    Storage, content, documents,
    git_repo::{self, CommitVersions},
};
use crate::models::result::{DiffLineInfo, DocumentVersionInfo, HistoricalDocument};
use anyhow::{Result, anyhow};
use csaf_trove_common::{Paginated, document_content::DocumentContent};
use csaf_trove_entity::{document, document_version};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, Statement, TransactionTrait, Value,
};
use std::{collections::BTreeSet, path::Path};
use tokio::{sync::mpsc, task::spawn_blocking};

/// Raw JSON of a stored document.
#[derive(Debug)]
pub enum RawDocument {
    /// The document text.
    Json(String),
    /// The document exceeds the requested limit and was not copied.
    TooLarge {
        /// Size of the document in bytes.
        size: usize,
    },
}

/// Maximum rows per multi-row insert, bounding SQLite parameter counts.
const INSERT_CHUNK: usize = 500;

/// Maximum URLs per `IN` list when refreshing version counts.
const URL_CHUNK: usize = 500;

/// Records versions from git history and refreshes `documents.version_count`.
///
/// With `since`, only commits after that checkpoint are scanned and existing
/// rows are kept; without it, all rows are rebuilt from the full history.
/// Inserting is idempotent, so retrying after a failed pass never duplicates.
pub async fn record_versions(
    db: &DatabaseConnection,
    repo_path: &Path,
    since: Option<&str>,
) -> Result<()> {
    if !repo_path.exists() {
        return Ok(());
    }
    let (tx, mut rx) = mpsc::channel::<CommitVersions>(16);
    let repo = repo_path.to_path_buf();
    let scan_since = since.map(String::from);
    let producer = spawn_blocking(move || {
        git_repo::collect_versions(&repo, scan_since.as_deref(), |commit| {
            tx.blocking_send(commit)
                .map_err(|_| anyhow!("version recording stopped"))
        })
    });

    let txn = db.begin().await?;
    if since.is_none() {
        txn.execute_unprepared("DELETE FROM document_versions")
            .await?;
    }
    let mut touched = BTreeSet::new();
    while let Some(commit) = rx.recv().await {
        for chunk in commit.documents.chunks(INSERT_CHUNK) {
            let placeholders = vec!["(?, ?, ?, ?, ?, ?, ?, ?)"; chunk.len()].join(", ");
            let mut values: Vec<Value> = Vec::with_capacity(chunk.len() * 8);
            for doc in chunk {
                values.extend([
                    doc.url.clone().into(),
                    commit.commit_id.clone().into(),
                    commit.timestamp.into(),
                    commit.message.clone().into(),
                    doc.blob_id.clone().into(),
                    doc.tracking.status.clone().into(),
                    doc.tracking.version.clone().into(),
                    doc.tracking.current_release_date.clone().into(),
                ]);
                touched.insert(doc.url.clone());
            }
            txn.execute_raw(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                format!(
                    "INSERT OR IGNORE INTO document_versions (url, commit_id, timestamp, message, blob_id, status, version, current_release_date) VALUES {placeholders}"
                ),
                values,
            ))
            .await?;
        }
    }
    producer.await??;

    if since.is_none() {
        refresh_version_counts(&txn, None).await?;
    } else {
        let touched: Vec<String> = touched.into_iter().collect();
        for chunk in touched.chunks(URL_CHUNK) {
            refresh_version_counts(&txn, Some(chunk)).await?;
        }
    }
    txn.commit().await?;
    Ok(())
}

/// Sets `version_count` from recorded versions, writing only rows whose count changed.
///
/// Documents without recorded history keep their stored count.
async fn refresh_version_counts(db: &impl ConnectionTrait, urls: Option<&[String]>) -> Result<()> {
    let (filter, values) = match urls {
        Some(urls) => (
            format!("WHERE url IN ({})", vec!["?"; urls.len()].join(", ")),
            urls.iter().cloned().map(Value::from).collect(),
        ),
        None => (String::new(), Vec::new()),
    };
    db.execute_raw(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        format!(
            "UPDATE documents SET version_count = counts.n
             FROM (SELECT url, COUNT(*) AS n FROM document_versions {filter} GROUP BY url) AS counts
             WHERE documents.url = counts.url AND documents.version_count != counts.n"
        ),
        values,
    ))
    .await?;
    Ok(())
}

/// Whether documents exist without any recorded versions, requiring a history backfill.
pub async fn versions_missing(db: &DatabaseConnection) -> Result<bool> {
    let has_versions = document_version::Entity::find().one(db).await?.is_some();
    let has_documents = document::Entity::find().one(db).await?.is_some();
    Ok(has_documents && !has_versions)
}

/// Converts a stored version row into its API representation.
fn version_info(row: document_version::Model, is_latest: bool) -> DocumentVersionInfo {
    DocumentVersionInfo {
        commit_id: row.commit_id,
        timestamp: row.timestamp,
        message: row.message,
        is_latest,
        status: row.status,
        version: row.version,
        current_release_date: row.current_release_date,
    }
}

/// Returns a page of a document's versions, newest first.
pub async fn list_versions(
    db: &DatabaseConnection,
    url: &str,
    offset: u64,
    limit: u64,
) -> Result<Paginated<DocumentVersionInfo>> {
    let query = document_version::Entity::find().filter(document_version::Column::Url.eq(url));
    let total = query.clone().count(db).await?;
    let rows = query
        .order_by_desc(document_version::Column::Id)
        .offset(offset)
        .limit(limit)
        .all(db)
        .await?;
    let items = rows
        .into_iter()
        .enumerate()
        .map(|(i, row)| version_info(row, offset == 0 && i == 0))
        .collect();
    Ok(Paginated {
        items,
        total,
        offset,
        limit,
    })
}

/// Looks up the version of a document recorded at a commit.
async fn find_version(
    db: &DatabaseConnection,
    url: &str,
    commit_id: &str,
) -> Result<Option<document_version::Model>> {
    Ok(document_version::Entity::find()
        .filter(document_version::Column::Url.eq(url))
        .filter(document_version::Column::CommitId.eq(commit_id))
        .one(db)
        .await?)
}

/// Returns the most recently recorded version of a document.
async fn latest_version(
    db: &DatabaseConnection,
    url: &str,
) -> Result<Option<document_version::Model>> {
    Ok(document_version::Entity::find()
        .filter(document_version::Column::Url.eq(url))
        .order_by_desc(document_version::Column::Id)
        .one(db)
        .await?)
}

/// Returns the version recorded directly after the given one, if any.
async fn next_newer_version(
    db: &DatabaseConnection,
    version: &document_version::Model,
) -> Result<Option<document_version::Model>> {
    Ok(document_version::Entity::find()
        .filter(document_version::Column::Url.eq(&version.url))
        .filter(document_version::Column::Id.gt(version.id))
        .order_by_asc(document_version::Column::Id)
        .one(db)
        .await?)
}

impl Storage {
    /// Records document versions from git, incrementally after `since` or rebuilt in full.
    pub async fn record_versions(&self, domain: &str, since: Option<&str>) -> Result<()> {
        let db = self.db.get(domain).await?;
        record_versions(&db, &self.repo_path(domain), since).await
    }

    /// Whether version history must be backfilled for documents stored before it was tracked.
    pub async fn versions_missing(&self, domain: &str) -> Result<bool> {
        let db = self.db.get(domain).await?;
        versions_missing(&db).await
    }

    /// Resolves a document's database connection and URL by tracking ID.
    async fn document_ref(
        &self,
        domain: &str,
        tracking_id: &str,
    ) -> Result<Option<(DatabaseConnection, String)>> {
        let Some(db) = self.db.get_if_exists(domain).await? else {
            return Ok(None);
        };
        let url = documents::document_url(&db, tracking_id).await?;
        Ok(url.map(|url| (db, url)))
    }

    /// Runs `f` on a blob in a blocking task, borrowing the content from libgit2 without a copy.
    async fn with_blob<T, F>(&self, domain: &str, blob_id: &str, f: F) -> Result<Option<T>>
    where
        F: FnOnce(&[u8]) -> T + Send + 'static,
        T: Send + 'static,
    {
        let repo_path = self.repo_path(domain);
        if !repo_path.exists() {
            return Ok(None);
        }
        let blob_id = blob_id.to_string();
        spawn_blocking(move || git_repo::with_blob(&repo_path, &blob_id, f)).await?
    }

    /// Returns a page of the version history for a document, newest first.
    pub async fn document_versions(
        &self,
        domain: &str,
        tracking_id: &str,
        offset: u64,
        limit: u64,
    ) -> Result<Option<Paginated<DocumentVersionInfo>>> {
        let Some((db, url)) = self.document_ref(domain, tracking_id).await? else {
            return Ok(None);
        };
        Ok(Some(list_versions(&db, &url, offset, limit).await?))
    }

    /// Computes a structured diff between a document version and its next newer version.
    pub async fn diff_document_versions(
        &self,
        domain: &str,
        tracking_id: &str,
        commit_id: &str,
    ) -> Result<Option<Vec<DiffLineInfo>>> {
        let Some((db, url)) = self.document_ref(domain, tracking_id).await? else {
            return Ok(None);
        };
        let Some(old) = find_version(&db, &url, commit_id).await? else {
            return Ok(None);
        };
        let Some(new) = next_newer_version(&db, &old).await? else {
            return Ok(None);
        };
        let repo_path = self.repo_path(domain);
        if !repo_path.exists() {
            return Ok(None);
        }
        spawn_blocking(move || {
            git_repo::with_blobs(
                &repo_path,
                &old.blob_id,
                &new.blob_id,
                git_repo::diff_documents,
            )
        })
        .await?
    }

    /// Reads a historical version of a document from git and extracts its metadata.
    pub async fn read_historical_document(
        &self,
        domain: &str,
        tracking_id: &str,
        commit_id: &str,
    ) -> Result<Option<HistoricalDocument>> {
        let Some((db, url)) = self.document_ref(domain, tracking_id).await? else {
            return Ok(None);
        };
        let Some(version) = find_version(&db, &url, commit_id).await? else {
            return Ok(None);
        };
        let commit_id = commit_id.to_string();
        self.with_blob(domain, &version.blob_id, move |blob| {
            super::extract_metadata_from_json(blob, &commit_id, version.timestamp)
        })
        .await?
        .transpose()
    }

    /// Reads the raw JSON of a document, either the current version or the one at `commit_id`.
    ///
    /// Documents larger than `max_bytes` are reported by size instead of being copied.
    pub async fn read_document_raw(
        &self,
        domain: &str,
        tracking_id: &str,
        commit_id: Option<&str>,
        max_bytes: usize,
    ) -> Result<Option<RawDocument>> {
        let Some((db, url)) = self.document_ref(domain, tracking_id).await? else {
            return Ok(None);
        };
        let version = match commit_id {
            Some(commit_id) => find_version(&db, &url, commit_id).await?,
            None => latest_version(&db, &url).await?,
        };
        let Some(version) = version else {
            return Ok(None);
        };
        self.with_blob(domain, &version.blob_id, move |blob| {
            if blob.len() > max_bytes {
                return Ok(RawDocument::TooLarge { size: blob.len() });
            }
            Ok(RawDocument::Json(std::str::from_utf8(blob)?.to_owned()))
        })
        .await?
        .transpose()
    }

    /// Reads the current version of a document from git and extracts its displayable content.
    pub async fn read_document_content(
        &self,
        domain: &str,
        tracking_id: &str,
    ) -> Result<Option<DocumentContent>> {
        let Some((db, url)) = self.document_ref(domain, tracking_id).await? else {
            return Ok(None);
        };
        let Some(version) = latest_version(&db, &url).await? else {
            return Ok(None);
        };
        self.with_blob(domain, &version.blob_id, content::extract_content)
            .await?
            .transpose()
    }
}

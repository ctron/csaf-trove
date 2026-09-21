use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Generic paginated response wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paginated<T> {
    /// The current page of results.
    pub items: Vec<T>,
    /// Total number of items matching the query.
    pub total: u64,
    /// Zero-based offset of the first item in this page.
    pub offset: u64,
    /// Maximum items per page.
    pub limit: u64,
}

/// A single data point for the sync history sparkline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPoint {
    /// Timestamp of the sync run.
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
    /// Number of documents changed.
    pub count: u64,
}

/// Summary of a single git commit from a provider's sync history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitInfo {
    /// Full commit SHA.
    pub id: String,
    /// Commit message.
    pub message: String,
    /// Commit timestamp.
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
    /// Number of documents changed in this commit.
    pub files_changed: usize,
}

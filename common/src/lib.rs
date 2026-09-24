#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub fn format_duration_hms(duration: Duration, truncate_seconds: bool) -> String {
    let total_secs = duration.as_secs();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let secs = total_secs % 60;

    if hours > 0 {
        if truncate_seconds {
            format!("{hours}h {minutes}m")
        } else {
            format!("{hours}h {minutes}m {secs}s")
        }
    } else if minutes > 0 {
        if truncate_seconds {
            format!("{minutes}m")
        } else {
            format!("{minutes}m {secs}s")
        }
    } else {
        format!("{secs}s")
    }
}

/// Pipeline phases that a sync job progresses through in order.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    strum::Display,
    strum::EnumIter,
    strum::AsRefStr,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum PipelinePhase {
    Checkout,
    Discover,
    Sync,
    Commit,
    Validate,
    Report,
}

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

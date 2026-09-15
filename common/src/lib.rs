use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

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

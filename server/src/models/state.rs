use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Persisted state tracking incremental sync progress for a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncState {
    /// Domain name of the provider.
    pub domain: String,
    /// Timestamp of the last successful sync.
    pub last_sync: Option<DateTime<Utc>>,
    /// Opaque token for incremental fetching (typically a timestamp).
    pub since_token: Option<DateTime<Utc>>,
    /// Number of documents fetched during the last sync.
    pub documents_synced: u64,
    /// Total number of documents known for this provider.
    pub documents_total: u64,
}

impl SyncState {
    /// Creates a new empty sync state for the given domain.
    pub fn new(domain: String) -> Self {
        Self {
            domain,
            last_sync: None,
            since_token: None,
            documents_synced: 0,
            documents_total: 0,
        }
    }
}

/// Snapshot of a running or recently completed pipeline job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobStatus {
    /// Current lifecycle phase of the job.
    pub status: JobPhase,
    /// When the job was started.
    pub started_at: DateTime<Utc>,
    /// When the job finished, if it has.
    pub completed_at: Option<DateTime<Utc>>,
    /// Human-readable label for the current pipeline stage.
    pub phase: Option<String>,
    /// Documents fetched so far.
    pub documents_synced: u64,
    /// Documents validated so far.
    pub documents_validated: u64,
    /// Total documents expected.
    pub documents_total: u64,
    /// Error message if the job failed.
    pub error: Option<String>,
}

/// Lifecycle phases of a pipeline job.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobPhase {
    /// Waiting to start.
    Pending,
    /// Currently executing.
    Running,
    /// Finished successfully.
    Completed,
    /// Finished with an error.
    Failed,
}

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Persisted state tracking incremental sync progress for a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncState {
    /// Domain name of the provider.
    pub domain: String,
    /// Timestamp of the last successful sync.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub last_sync: Option<OffsetDateTime>,
    /// Opaque token for incremental fetching (typically a timestamp).
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub since_token: Option<OffsetDateTime>,
    /// Number of documents fetched during the last sync.
    pub documents_synced: u64,
    /// Number of documents validated during the last run.
    #[serde(default)]
    pub documents_validated: u64,
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
            documents_validated: 0,
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
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    /// When the job finished, if it has.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub completed_at: Option<OffsetDateTime>,
    /// Human-readable label for the current pipeline stage.
    pub phase: Option<String>,
    /// Documents fetched so far.
    pub documents_synced: u64,
    /// Documents validated so far.
    pub documents_validated: u64,
    /// Total documents expected.
    pub documents_total: u64,
    /// Total distributions for the current phase.
    pub distributions_total: u64,
    /// Current distribution being processed (1-based).
    pub distribution_index: u64,
    /// Documents processed in the current distribution.
    pub distribution_documents_current: u64,
    /// Total documents expected in the current distribution.
    pub distribution_documents_total: u64,
    /// Error message if the job failed.
    pub error: Option<String>,
    /// When the previous run completed (carried forward when a new run starts).
    #[serde(skip)]
    pub last_completed_at: Option<OffsetDateTime>,
    /// When the current pipeline phase began (used for ETA calculation).
    #[serde(skip)]
    pub phase_started_at: Option<OffsetDateTime>,
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

use serde::{Deserialize, Serialize};

/// Totals from the latest provider summaries and managed storage files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Overview {
    /// Providers listed on the providers page, including those awaiting sync.
    pub provider_count: u64,
    /// Document records in the latest completed provider summaries.
    pub document_count: u64,
    /// Document records with retrieval errors in those summaries.
    pub retrieval_errors: u64,
    /// File sizes in the managed persistent storage directories.
    pub storage: StorageUsage,
}

/// Logical file sizes in bytes, excluding temporary worktrees and aggregator output.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StorageUsage {
    /// Git repositories, including document history and metadata.
    pub repositories: u64,
    /// Validation databases, summaries, and SQLite journal files.
    pub results: u64,
    /// Persisted synchronization state.
    pub state: u64,
    /// Historical validation metrics.
    pub metrics: u64,
}

impl StorageUsage {
    /// Returns the combined size of all managed storage categories.
    pub fn total(&self) -> u64 {
        self.repositories + self.results + self.state + self.metrics
    }
}

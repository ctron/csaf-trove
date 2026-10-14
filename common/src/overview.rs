use crate::document_checks::DocumentCheckSummary;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

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
    /// Aggregate validation results across providers.
    #[serde(default)]
    pub validation: ValidationOverview,
    /// Provider counts by health state.
    #[serde(default)]
    pub providers: ProviderHealth,
    /// Current scheduler activity.
    #[serde(default)]
    pub sync: SyncActivity,
    /// Changes compared to earlier daily snapshots.
    #[serde(default)]
    pub trends: Trends,
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

/// Validation results combined across all providers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ValidationOverview {
    /// Basic profile pass rate weighted by test count (0.0–1.0).
    pub basic_pass_rate: Option<f64>,
    /// Extended profile pass rate weighted by test count (0.0–1.0).
    pub extended_pass_rate: Option<f64>,
    /// Full profile pass rate weighted by test count (0.0–1.0).
    pub full_pass_rate: Option<f64>,
    /// Summed essential check outcomes.
    pub checks: DocumentCheckSummary,
}

/// Provider counts by health state; a provider may appear in several issue categories.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderHealth {
    /// Synced providers without retrieval errors or essential check issues.
    pub healthy: u64,
    /// Providers with at least one retrieval error.
    pub with_retrieval_errors: u64,
    /// Providers with essential check failures, warnings, or missing inputs.
    pub with_check_issues: u64,
    /// Enabled providers without a completed summary.
    pub awaiting_first_sync: u64,
    /// Providers whose latest summary is older than twice the sync interval.
    pub stale: u64,
    /// Age in seconds after which a summary is considered stale.
    pub stale_after_seconds: u64,
}

/// Scheduler job counts from the in-memory job status map.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncActivity {
    /// Jobs currently running.
    pub running: u64,
    /// Jobs whose latest run failed.
    pub failed: u64,
    /// Most recent job completion time.
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_completed_at: Option<OffsetDateTime>,
}

/// Changes over fixed windows, absent when no snapshot is old enough.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Trends {
    /// Change over the last 7 days.
    pub week: Option<TrendDelta>,
    /// Change over the last 30 days.
    pub month: Option<TrendDelta>,
}

/// Difference between current totals and an earlier snapshot of the same providers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrendDelta {
    /// Providers that have a snapshot old enough to compare.
    pub providers: u64,
    /// Change in document count.
    pub documents: i64,
    /// Change in documents with retrieval errors.
    pub retrieval_errors: i64,
    /// Change in basic profile pass rate (fraction, not percentage points).
    pub basic_pass_rate: Option<f64>,
}

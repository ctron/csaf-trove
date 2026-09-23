use serde::{Deserialize, Serialize};

/// Rolling time series of daily validation snapshots for a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsTimeSeries {
    /// Domain name of the provider.
    pub provider: String,
    /// Daily validation snapshots, oldest first.
    pub entries: Vec<MetricsEntry>,
}

/// A single daily validation snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsEntry {
    /// Date of the snapshot (`YYYY-MM-DD`).
    pub date: String,
    /// Total documents evaluated.
    pub document_count: u64,
    /// Basic profile metrics for this day.
    pub basic: Option<MetricsProfileEntry>,
    /// Extended profile metrics for this day.
    pub extended: Option<MetricsProfileEntry>,
    /// Full profile metrics for this day.
    pub full: Option<MetricsProfileEntry>,
    /// Signature metrics for this day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signatures: Option<MetricsSignatureEntry>,
    /// Number of documents with retrieval errors.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub retrieval_errors: u64,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

/// Profile-level validation counts for a metrics entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsProfileEntry {
    /// Total tests passed across all documents.
    pub valid: u64,
    /// Total tests failed across all documents.
    pub invalid: u64,
    /// Ratio of valid to total (0.0–1.0).
    pub pass_rate: f64,
}

/// Signature validation counts for a metrics entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsSignatureEntry {
    /// Documents with a valid signature.
    pub valid: u64,
    /// Documents with an invalid signature.
    pub invalid: u64,
    /// Documents with no signature.
    pub missing: u64,
}

impl MetricsTimeSeries {
    /// Creates an empty time series for the given provider.
    pub fn new(provider: String) -> Self {
        Self {
            provider,
            entries: Vec::new(),
        }
    }

    /// Adds an entry and trims to a 365-day rolling window.
    pub fn append(&mut self, entry: MetricsEntry) {
        self.entries.push(entry);
        self.trim(365);
    }

    fn trim(&mut self, max_entries: usize) {
        if self.entries.len() > max_entries {
            let drain_count = self.entries.len() - max_entries;
            self.entries.drain(..drain_count);
        }
    }
}

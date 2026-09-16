use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSummary {
    pub provider: String,
    pub publisher_name: Option<String>,
    pub validated_at: String,
    pub document_count: u64,
    pub profiles: ProfileResults,
    #[serde(default)]
    pub signatures: Option<SignatureSummary>,
    pub top_failing_tests: Vec<FailingTest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileResults {
    pub basic: Option<ProfileSummary>,
    pub extended: Option<ProfileSummary>,
    pub full: Option<ProfileSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileSummary {
    pub valid: u64,
    pub invalid: u64,
    pub pass_rate: f64,
}

/// Aggregate signature validation counts for a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureSummary {
    /// Documents with a valid signature.
    pub valid: u64,
    /// Documents with an invalid signature.
    pub invalid: u64,
    /// Documents with no signature file.
    pub missing: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailingTest {
    pub test_id: String,
    pub count: u64,
    pub severity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDetail {
    pub summary: ProviderSummary,
    pub metrics: Option<MetricsTimeSeries>,
    #[serde(default)]
    pub history: Vec<csaf_trove_common::CommitInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsTimeSeries {
    pub provider: String,
    pub entries: Vec<MetricsEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsEntry {
    pub date: String,
    pub document_count: u64,
    pub basic: Option<MetricsProfileEntry>,
    pub extended: Option<MetricsProfileEntry>,
    pub full: Option<MetricsProfileEntry>,
    #[serde(default)]
    pub signatures: Option<MetricsSignatureEntry>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsProfileEntry {
    pub valid: u64,
    pub invalid: u64,
    pub pass_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobStatus {
    pub status: String,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub phase: Option<String>,
    pub documents_synced: u64,
    pub documents_validated: u64,
    pub documents_total: u64,
    pub error: Option<String>,
    /// Elapsed seconds (running) or total seconds (completed/failed).
    pub duration_seconds: Option<f64>,
    /// Pre-formatted ETA string (e.g. "~5m 30s"), only while running.
    #[serde(default)]
    pub eta: Option<String>,
    /// When the provider last completed a sync (ISO 8601).
    #[serde(default)]
    pub last_run: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentValidation {
    pub tracking_id: String,
    pub title: String,
    pub url: String,
    pub profiles: DocumentProfileResults,
    pub signature_error: Option<String>,
    pub signature_present: bool,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub publisher_name: Option<String>,
    #[serde(default)]
    pub initial_release_date: Option<String>,
    #[serde(default)]
    pub current_release_date: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub aggregate_severity: Option<String>,
    #[serde(default)]
    pub csaf_version: Option<String>,
    #[serde(default)]
    pub revision_history: Vec<RevisionEntry>,
}

/// A single entry in the CSAF revision history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevisionEntry {
    /// Revision version number.
    pub number: String,
    /// Date of this revision.
    pub date: String,
    /// Short description of the changes.
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentProfileResults {
    pub basic: Option<DocumentProfileDetail>,
    pub extended: Option<DocumentProfileDetail>,
    pub full: Option<DocumentProfileDetail>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentProfileDetail {
    pub passed: bool,
    pub error_count: u64,
    pub warning_count: u64,
    pub info_count: u64,
    pub failing_tests: Vec<DocumentCheckFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentCheckFailure {
    pub test_id: String,
    pub message: String,
    pub severity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedDocuments {
    pub items: Vec<DocumentValidation>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentVersionInfo {
    pub commit_id: String,
    pub timestamp: i64,
    pub message: String,
    pub is_latest: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalDocument {
    pub tracking_id: String,
    pub title: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub publisher_name: Option<String>,
    #[serde(default)]
    pub initial_release_date: Option<String>,
    #[serde(default)]
    pub current_release_date: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub revision: Option<String>,
    #[serde(default)]
    pub aggregate_severity: Option<String>,
    #[serde(default)]
    pub csaf_version: Option<String>,
    #[serde(default)]
    pub revision_history: Vec<RevisionEntry>,
    pub commit_id: String,
    pub timestamp: i64,
}

/// URL-encodes a path segment so that characters like `/` and `:` are percent-escaped.
pub fn encode_path_segment(s: &str) -> String {
    js_sys::encode_uri_component(s)
        .as_string()
        .unwrap_or_default()
}

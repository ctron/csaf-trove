use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSummary {
    pub provider: String,
    pub publisher_name: Option<String>,
    pub validated_at: String,
    pub document_count: u64,
    pub profiles: ProfileResults,
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
    pub failing_tests: Vec<DocumentCheckFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentCheckFailure {
    pub test_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedDocuments {
    pub items: Vec<DocumentValidation>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

use serde::{Deserialize, Serialize};

/// Validation summary for a single CSAF provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSummary {
    /// Domain name of the provider.
    pub provider: String,
    /// Publisher name from the CSAF metadata, if available.
    pub publisher_name: Option<String>,
    /// When this summary was generated.
    pub validated_at: chrono::DateTime<chrono::Utc>,
    /// Total number of CSAF documents evaluated.
    pub document_count: u64,
    /// Pass/fail breakdown per CSAF profile.
    pub profiles: ProfileResults,
    /// Most frequently failing test IDs across all documents.
    pub top_failing_tests: Vec<FailingTest>,
}

/// Validation results per CSAF profile level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileResults {
    /// Results for the basic (mandatory) profile.
    pub basic: Option<ProfileSummary>,
    /// Results for the extended (mandatory + recommended) profile.
    pub extended: Option<ProfileSummary>,
    /// Results for the full (all tests) profile.
    pub full: Option<ProfileSummary>,
}

/// Aggregate pass/fail counts for a single profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileSummary {
    /// Number of documents passing all tests in this profile.
    pub valid: u64,
    /// Number of documents failing at least one test in this profile.
    pub invalid: u64,
    /// Ratio of valid to total (0.0–1.0).
    pub pass_rate: f64,
}

/// A frequently failing validation test across documents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailingTest {
    /// Identifier of the failing test (e.g. `6.1.27.5`).
    pub test_id: String,
    /// Number of documents failing this test.
    pub count: u64,
    /// Severity level of the test failure.
    pub severity: String,
}

/// Validation results for a single CSAF document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentValidation {
    /// CSAF tracking ID (e.g. `RHSA-2024:1234`).
    pub tracking_id: String,
    /// Document title from the CSAF metadata.
    pub title: String,
    /// URL where the document was discovered.
    pub url: String,
    /// Per-profile validation results.
    pub profiles: DocumentProfileResults,
    /// Signature validation error message, if the signature was invalid.
    pub signature_error: Option<String>,
    /// Whether a signature file was present for this document.
    pub signature_present: bool,
    /// Document category (e.g. `csaf_security_advisory`, `csaf_vex`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Name of the document publisher.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_name: Option<String>,
    /// Date of the initial release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_release_date: Option<String>,
    /// Date of the current (latest) release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_release_date: Option<String>,
    /// Document status (`draft`, `final`, `interim`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Document tracking version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// Aggregate severity text (e.g. `critical`, `important`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate_severity: Option<String>,
    /// CSAF specification version (e.g. `2.0`, `2.1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub csaf_version: Option<String>,
}

/// Per-profile failure information for a single document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentProfileResults {
    /// Failures in the basic profile.
    pub basic: Option<DocumentProfileDetail>,
    /// Failures in the extended profile.
    pub extended: Option<DocumentProfileDetail>,
    /// Failures in the full profile.
    pub full: Option<DocumentProfileDetail>,
}

/// Detail of test failures for one profile on one document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentProfileDetail {
    /// Whether this profile passed (zero failures).
    pub passed: bool,
    /// Total number of check errors for this profile.
    pub error_count: u64,
    /// Individual failing tests.
    pub failing_tests: Vec<DocumentCheckFailure>,
}

/// A single check failure on a document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentCheckFailure {
    /// Test identifier (e.g. `6.1.27.5`).
    pub test_id: String,
    /// Human-readable failure description.
    pub message: String,
}

/// A version of a document from the git history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentVersionInfo {
    /// Commit SHA where this version was recorded.
    pub commit_id: String,
    /// Commit timestamp as Unix seconds.
    pub timestamp: i64,
    /// Commit message.
    pub message: String,
    /// Whether this is the most recent (HEAD) version.
    pub is_latest: bool,
}

/// Metadata extracted from a historical CSAF document blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoricalDocument {
    /// CSAF tracking ID.
    pub tracking_id: String,
    /// Document title.
    pub title: String,
    /// Document category.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Name of the document publisher.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publisher_name: Option<String>,
    /// Date of the initial release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_release_date: Option<String>,
    /// Date of the current (latest) release.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_release_date: Option<String>,
    /// Document status (`draft`, `final`, `interim`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Document tracking version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// Aggregate severity text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aggregate_severity: Option<String>,
    /// CSAF specification version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub csaf_version: Option<String>,
    /// Commit SHA this version is from.
    pub commit_id: String,
    /// Commit timestamp as Unix seconds.
    pub timestamp: i64,
}

/// Paginated response wrapper for document validation results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedDocuments {
    /// The current page of document results.
    pub items: Vec<DocumentValidation>,
    /// Total number of documents matching the query.
    pub total: u64,
    /// Zero-based offset of the first item in this page.
    pub offset: u64,
    /// Maximum items per page.
    pub limit: u64,
}

use super::metrics::MetricsTimeSeries;
use csaf_trove_common::CommitInfo;
use csaf_trove_common::document_checks::{DocumentCheckSummary, DocumentChecks};
use serde::{Deserialize, Serialize};

/// Combined detail view for a provider including summary, metrics, and sync history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDetail {
    /// Validation summary.
    pub summary: ProviderSummary,
    /// Historical metrics time series.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metrics: Option<MetricsTimeSeries>,
    /// Recent sync history from git commits.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<CommitInfo>,
    /// Per-distribution health breakdown.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub distributions: Vec<DistributionHealth>,
    /// Operator note from the source configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Health metrics for a single distribution (directory or ROLIE feed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistributionHealth {
    /// Human-readable label (typically the URL path).
    pub label: String,
    /// Kind: `"directory"`, `"rolie"`, or `"directory+rolie"`.
    pub kind: String,
    /// Distribution URL (directory_url or feed URL).
    pub url: String,
    /// TLP labels of the ROLIE feeds in this distribution.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tlp_labels: Vec<String>,
    /// Number of documents matched to this distribution.
    pub document_count: u64,
    /// Number of documents with essential check failures, warnings, or missing inputs; excludes CSAF tests.
    pub check_issues: u64,
    /// Whether this distribution was skipped by configuration.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skipped: bool,
    /// Distribution-level error (e.g. 403 on a restricted ROLIE feed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distribution_error: Option<String>,
    /// Basic profile pass rate (0.0–1.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basic_pass_rate: Option<f64>,
    /// Extended profile pass rate (0.0–1.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extended_pass_rate: Option<f64>,
    /// Full profile pass rate (0.0–1.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_pass_rate: Option<f64>,
}

/// Validation summary for a single CSAF provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSummary {
    /// Independent essential document outcome counts; absent in old cached summaries.
    #[serde(default)]
    pub checks: Option<DocumentCheckSummary>,
    /// Domain name of the provider.
    pub provider: String,
    /// Publisher name from the CSAF metadata, if available.
    pub publisher_name: Option<String>,
    /// When this summary was generated.
    #[serde(with = "time::serde::rfc3339")]
    pub validated_at: time::OffsetDateTime,
    /// Total number of CSAF documents evaluated.
    pub document_count: u64,
    /// Pass/fail breakdown per CSAF profile.
    pub profiles: ProfileResults,
    /// Signature validation breakdown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signatures: Option<SignatureSummary>,
    /// Most frequently failing test IDs across all documents.
    pub top_failing_tests: Vec<FailingTest>,
    /// Number of documents with retrieval errors.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub retrieval_errors: u64,
    /// Operator note from the source configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

fn default_version_count() -> u32 {
    1
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
    /// Total tests passed across all documents in this profile.
    pub valid: u64,
    /// Total tests failed across all documents in this profile.
    pub invalid: u64,
    /// Ratio of valid to total (0.0–1.0).
    pub pass_rate: f64,
}

/// Aggregate signature validation counts for a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureSummary {
    /// Documents with a valid signature (present, no error).
    pub valid: u64,
    /// Documents with an invalid signature (present, with error).
    pub invalid: u64,
    /// Documents with no signature file.
    pub missing: u64,
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
    /// Independent essential document outcomes.
    #[serde(default)]
    pub checks: DocumentChecks,
    /// CSAF tracking ID (e.g. `RHSA-2024:1234`).
    pub tracking_id: String,
    /// Document title from the CSAF metadata.
    pub title: String,
    /// URL where the document was discovered.
    pub url: String,
    /// Per-profile validation results.
    pub profiles: DocumentProfileResults,
    /// Legacy combined parse/signature/digest diagnostic; use `checks` for independent results.
    pub signature_error: Option<String>,
    /// Legacy digest warning; use `checks.digest` for its outcome and diagnostic.
    #[serde(default)]
    pub signature_warning: Option<String>,
    /// Legacy combined signature/checksum presence; use `checks` for separate outcomes.
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
    /// Revision history entries from the CSAF tracking section.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revision_history: Vec<RevisionEntry>,
    /// Number of distinct git versions (commits where the document blob changed).
    #[serde(default = "default_version_count")]
    pub version_count: u32,
    /// Retrieval error message from the last sync attempt, if the fetch failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retrieval_error: Option<String>,
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
    /// Number of mandatory test failures.
    pub error_count: u64,
    /// Number of optional/recommended test failures.
    pub warning_count: u64,
    /// Number of informational test failures.
    pub info_count: u64,
    /// Total number of tests in this profile for the document's CSAF version.
    #[serde(default)]
    pub total_tests: u64,
    /// Number of distinct test IDs that failed.
    #[serde(default)]
    pub failing_test_count: u64,
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
    /// Severity level derived from the test section (`error`, `warning`, or `info`).
    pub severity: String,
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
    /// CSAF tracking status at this version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// CSAF tracking version at this version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// CSAF tracking current release date at this version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_release_date: Option<String>,
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
    /// Revision history entries from the CSAF tracking section.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revision_history: Vec<RevisionEntry>,
    /// Commit SHA this version is from.
    pub commit_id: String,
    /// Commit timestamp as Unix seconds.
    pub timestamp: i64,
}

/// Tag indicating how a diff line relates to the comparison.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffTag {
    /// Line is unchanged between versions.
    Equal,
    /// Line was added in the newer version.
    Insert,
    /// Line was removed from the older version.
    Delete,
}

/// A single line in a structured diff between two document versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffLineInfo {
    /// Whether this line is unchanged, inserted, or deleted.
    pub tag: DiffTag,
    /// The line content (without trailing newline).
    pub content: String,
}

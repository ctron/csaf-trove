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

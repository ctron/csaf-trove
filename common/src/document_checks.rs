//! Independent outcomes for the essential stages of document processing.

use serde::{Deserialize, Serialize};

/// Result of an essential document check.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// The check completed successfully.
    Passed,
    /// The check failed.
    Failed,
    /// A digest matched but another supplied digest failed.
    Warning,
    /// No signature or digest was supplied.
    Missing,
    /// The check did not run, or its old result cannot be determined.
    #[default]
    NotEvaluated,
}

/// An outcome with an optional diagnostic message.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckOutcome {
    /// Recorded outcome.
    pub status: CheckStatus,
    /// Diagnostic explaining a failure, warning, or unavailable result.
    pub message: Option<String>,
}

impl CheckOutcome {
    /// Creates an outcome without a diagnostic.
    pub fn new(status: CheckStatus) -> Self {
        Self {
            status,
            message: None,
        }
    }

    /// Creates a failed outcome preserving its diagnostic.
    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            status: CheckStatus::Failed,
            message: Some(message.into()),
        }
    }
}

/// Independent retrieval, parsing, OpenPGP, and digest results or aggregate counts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DocumentChecks<T = CheckOutcome> {
    /// Latest retrieval attempt.
    pub retrieval: T,
    /// Parsing of the retrieved CSAF document.
    pub parsing: T,
    /// OpenPGP authenticity verification.
    pub signature: T,
    /// Combined SHA-256 and SHA-512 verification.
    pub digest: T,
}

/// Document counts for every possible check outcome.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckCounts {
    /// Checks that passed.
    pub passed: u64,
    /// Checks that failed.
    pub failed: u64,
    /// Checks that passed with warnings.
    pub warning: u64,
    /// Checks with no supplied input.
    pub missing: u64,
    /// Checks that were not evaluated or have unknown historical results.
    pub not_evaluated: u64,
}

impl CheckCounts {
    /// Adds documents with the given outcome.
    pub fn add(&mut self, status: CheckStatus, count: u64) {
        *match status {
            CheckStatus::Passed => &mut self.passed,
            CheckStatus::Failed => &mut self.failed,
            CheckStatus::Warning => &mut self.warning,
            CheckStatus::Missing => &mut self.missing,
            CheckStatus::NotEvaluated => &mut self.not_evaluated,
        } += count;
    }
}

/// Independent outcome counts for a provider or daily snapshot.
pub type DocumentCheckSummary = DocumentChecks<CheckCounts>;

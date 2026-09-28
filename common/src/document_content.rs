//! Content extracted from a CSAF document for display in the dashboard.

use serde::{Deserialize, Serialize};

/// Displayable content of the current version of a CSAF document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DocumentContent {
    /// Upper-cased TLP label from the distribution section.
    #[serde(default)]
    pub tlp: Option<String>,
    /// Publisher of the document.
    #[serde(default)]
    pub publisher: Option<Publisher>,
    /// Document-level notes.
    #[serde(default)]
    pub notes: Vec<Note>,
    /// Document-level references.
    #[serde(default)]
    pub references: Vec<Reference>,
    /// Vulnerabilities described by the document.
    #[serde(default)]
    pub vulnerabilities: Vec<Vulnerability>,
}

/// Publisher of a CSAF document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Publisher {
    /// Name of the publisher.
    pub name: String,
    /// Publisher category, such as `vendor` or `coordinator`.
    #[serde(default)]
    pub category: Option<String>,
    /// Namespace URL of the publisher.
    #[serde(default)]
    pub namespace: Option<String>,
    /// Contact details of the publisher.
    #[serde(default)]
    pub contact_details: Option<String>,
    /// Issuing authority statement.
    #[serde(default)]
    pub issuing_authority: Option<String>,
}

/// A note attached to a document or vulnerability.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// Note category, such as `summary` or `details`.
    #[serde(default)]
    pub category: Option<String>,
    /// Optional note title.
    #[serde(default)]
    pub title: Option<String>,
    /// Note text.
    pub text: String,
}

/// A reference attached to a document or vulnerability.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    /// Reference category, either `self` or `external`.
    #[serde(default)]
    pub category: Option<String>,
    /// Description of the reference.
    pub summary: String,
    /// Target URL.
    pub url: String,
}

/// A CWE classification of a vulnerability.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cwe {
    /// CWE identifier, such as `CWE-79`.
    pub id: String,
    /// CWE name.
    #[serde(default)]
    pub name: Option<String>,
}

/// Number of products listed under one product status.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductStatusCount {
    /// Product status key, such as `known_affected` or `fixed`.
    pub status: String,
    /// Number of products with that status.
    pub count: usize,
}

/// A vulnerability described by a CSAF document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Vulnerability {
    /// CVE identifier.
    #[serde(default)]
    pub cve: Option<String>,
    /// Other identifiers of the vulnerability.
    #[serde(default)]
    pub ids: Vec<String>,
    /// Vulnerability title.
    #[serde(default)]
    pub title: Option<String>,
    /// CWE classifications.
    #[serde(default)]
    pub cwes: Vec<Cwe>,
    /// Severity of the highest CVSS score.
    #[serde(default)]
    pub severity: Option<String>,
    /// Highest CVSS base score.
    #[serde(default)]
    pub score: Option<f64>,
    /// Product counts per product status.
    #[serde(default)]
    pub product_status: Vec<ProductStatusCount>,
    /// Vulnerability-level notes.
    #[serde(default)]
    pub notes: Vec<Note>,
    /// Vulnerability-level references.
    #[serde(default)]
    pub references: Vec<Reference>,
}

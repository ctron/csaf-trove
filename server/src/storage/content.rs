//! Extraction of displayable content from raw CSAF documents.
mod test;

use crate::models::result::VulnerabilityEntry;
use anyhow::Result;
use csaf_trove_common::document_content::{
    Cwe, DocumentContent, Note, ProductStatusCount, Publisher, Reference, Vulnerability,
};
use serde::Deserialize;
use serde_json::Value;

/// Extracts displayable content from raw CSAF 2.0 or 2.1 JSON.
///
/// Missing or malformed optional sections are skipped instead of failing.
pub fn extract_content(json: &[u8]) -> Result<DocumentContent> {
    let val: Value = serde_json::from_slice(json)?;
    let doc = &val["document"];

    Ok(DocumentContent {
        tlp: string(&doc["distribution"]["tlp"]["label"]).map(|s| s.to_uppercase()),
        publisher: publisher(&doc["publisher"]),
        notes: notes(&doc["notes"]),
        references: references(&doc["references"]),
        vulnerabilities: array(&val["vulnerabilities"]).map(vulnerability).collect(),
    })
}

/// Only the `vulnerabilities` section of a CSAF document; everything else is skipped while parsing.
#[derive(Deserialize)]
struct VulnerabilitiesOnly {
    /// Raw vulnerability entries.
    #[serde(default)]
    vulnerabilities: Vec<Value>,
}

/// Extracts the indexable identifiers and scores of every vulnerability in raw CSAF JSON.
///
/// Vulnerabilities without any CVE or other identifier are skipped.
pub fn extract_vulnerability_index(json: &[u8]) -> Result<Vec<VulnerabilityEntry>> {
    let doc: VulnerabilitiesOnly = serde_json::from_slice(json)?;
    Ok(doc
        .vulnerabilities
        .iter()
        .filter_map(|vuln| {
            let cve = string(&vuln["cve"]).map(|cve| normalize_cve(&cve));
            let ids: Vec<String> = array(&vuln["ids"])
                .filter_map(|id| string(&id["text"]))
                .collect();
            if cve.is_none() && ids.is_empty() {
                return None;
            }
            let (score, severity) = max_cvss(vuln);
            Some(VulnerabilityEntry {
                cve,
                ids,
                title: string(&vuln["title"]),
                score,
                severity,
            })
        })
        .collect())
}

/// Normalizes a CVE identifier for exact matching: trimmed and upper-cased.
pub fn normalize_cve(cve: &str) -> String {
    cve.trim().to_uppercase()
}

/// Returns the value as an owned string, if it is a string.
fn string(value: &Value) -> Option<String> {
    value.as_str().map(String::from)
}

/// Iterates over the value's elements, or nothing if it is not an array.
fn array(value: &Value) -> impl Iterator<Item = &Value> {
    value.as_array().into_iter().flatten()
}

/// Extracts the publisher section.
fn publisher(value: &Value) -> Option<Publisher> {
    Some(Publisher {
        name: string(&value["name"])?,
        category: string(&value["category"]),
        namespace: string(&value["namespace"]),
        contact_details: string(&value["contact_details"]),
        issuing_authority: string(&value["issuing_authority"]),
    })
}

/// Extracts a list of notes, skipping entries without text.
fn notes(value: &Value) -> Vec<Note> {
    array(value)
        .filter_map(|note| {
            Some(Note {
                category: string(&note["category"]),
                title: string(&note["title"]),
                text: string(&note["text"])?,
            })
        })
        .collect()
}

/// Extracts a list of references, skipping entries without a URL.
fn references(value: &Value) -> Vec<Reference> {
    array(value)
        .filter_map(|reference| {
            Some(Reference {
                category: string(&reference["category"]),
                summary: string(&reference["summary"]).unwrap_or_default(),
                url: string(&reference["url"])?,
            })
        })
        .collect()
}

/// Extracts a CWE entry, skipping entries without an ID.
fn cwe(value: &Value) -> Option<Cwe> {
    Some(Cwe {
        id: string(&value["id"])?,
        name: string(&value["name"]),
    })
}

/// Returns the CVSS objects of a vulnerability, from 2.0 `scores` or 2.1 `metrics`.
fn cvss_entries(vuln: &Value) -> impl Iterator<Item = &Value> {
    let scores = array(&vuln["scores"]);
    let metrics = array(&vuln["metrics"]).map(|metric| &metric["content"]);
    scores
        .chain(metrics)
        .flat_map(|entry| ["cvss_v4", "cvss_v3", "cvss_v2"].map(|key| &entry[key]))
        .filter(|cvss| cvss.is_object())
}

/// Returns the highest CVSS base score of a vulnerability and its severity.
fn max_cvss(vuln: &Value) -> (Option<f64>, Option<String>) {
    cvss_entries(vuln)
        .filter_map(|cvss| Some((cvss["baseScore"].as_f64()?, string(&cvss["baseSeverity"]))))
        .max_by(|(a, _), (b, _)| a.total_cmp(b))
        .map_or((None, None), |(score, severity)| (Some(score), severity))
}

/// Extracts a single vulnerability.
fn vulnerability(vuln: &Value) -> Vulnerability {
    let (score, severity) = max_cvss(vuln);

    let cwes = match vuln.get("cwes") {
        Some(cwes) => array(cwes).filter_map(cwe).collect(),
        None => vuln.get("cwe").and_then(cwe).into_iter().collect(),
    };

    let product_status = vuln["product_status"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(status, products)| {
            Some(ProductStatusCount {
                status: status.clone(),
                count: products.as_array()?.len(),
            })
        })
        .collect();

    Vulnerability {
        cve: string(&vuln["cve"]),
        ids: array(&vuln["ids"])
            .filter_map(|id| string(&id["text"]))
            .collect(),
        title: string(&vuln["title"]),
        cwes,
        severity,
        score,
        product_status,
        notes: notes(&vuln["notes"]),
        references: references(&vuln["references"]),
    }
}

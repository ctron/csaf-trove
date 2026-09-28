//! Content extraction tests for CSAF 2.0 and 2.1 documents.

#![cfg(test)]

use super::extract_content;
use csaf_trove_common::document_content::{Cwe, ProductStatusCount};
use serde_json::json;

/// Serializes a JSON value and extracts its content.
fn extract(value: serde_json::Value) -> csaf_trove_common::document_content::DocumentContent {
    extract_content(&serde_json::to_vec(&value).unwrap()).unwrap()
}

#[test]
fn csaf_2_0() {
    let content = extract(json!({
        "document": {
            "category": "csaf_security_advisory",
            "csaf_version": "2.0",
            "distribution": { "tlp": { "label": "WHITE" } },
            "publisher": {
                "name": "Example",
                "category": "vendor",
                "namespace": "https://example.com",
                "contact_details": "security@example.com"
            },
            "notes": [
                { "category": "summary", "title": "Summary", "text": "Something happened" },
                { "category": "details" }
            ],
            "references": [
                { "category": "self", "summary": "Self", "url": "https://example.com/a.json" }
            ]
        },
        "vulnerabilities": [{
            "cve": "CVE-2024-0001",
            "ids": [{ "system_name": "Example", "text": "EX-1" }],
            "title": "Overflow",
            "cwe": { "id": "CWE-787", "name": "Out-of-bounds Write" },
            "scores": [
                { "products": ["p1"], "cvss_v3": { "baseScore": 5.0, "baseSeverity": "MEDIUM" } },
                { "products": ["p2"], "cvss_v3": { "baseScore": 9.8, "baseSeverity": "CRITICAL" } }
            ],
            "product_status": { "known_affected": ["p1", "p2"], "fixed": ["p3"] },
            "notes": [{ "category": "description", "text": "Details" }]
        }]
    }));

    assert_eq!(content.tlp.as_deref(), Some("WHITE"));
    let publisher = content.publisher.unwrap();
    assert_eq!(publisher.name, "Example");
    assert_eq!(publisher.category.as_deref(), Some("vendor"));
    assert_eq!(
        publisher.contact_details.as_deref(),
        Some("security@example.com")
    );
    assert_eq!(content.notes.len(), 1);
    assert_eq!(content.references.len(), 1);

    let vuln = &content.vulnerabilities[0];
    assert_eq!(vuln.cve.as_deref(), Some("CVE-2024-0001"));
    assert_eq!(vuln.ids, ["EX-1"]);
    assert_eq!(
        vuln.cwes,
        [Cwe {
            id: "CWE-787".into(),
            name: Some("Out-of-bounds Write".into()),
        }]
    );
    assert_eq!(vuln.score, Some(9.8));
    assert_eq!(vuln.severity.as_deref(), Some("CRITICAL"));
    assert_eq!(
        vuln.product_status,
        [
            ProductStatusCount {
                status: "known_affected".into(),
                count: 2,
            },
            ProductStatusCount {
                status: "fixed".into(),
                count: 1,
            },
        ]
    );
    assert_eq!(vuln.notes.len(), 1);
}

#[test]
fn csaf_2_1() {
    let content = extract(json!({
        "$schema": "https://docs.oasis-open.org/csaf/csaf/v2.1/schema/csaf.json",
        "document": {
            "csaf_version": "2.1",
            "distribution": { "tlp": { "label": "clear" } },
            "publisher": { "name": "Example", "category": "coordinator" }
        },
        "vulnerabilities": [{
            "cve": "CVE-2025-0002",
            "cwes": [
                { "id": "CWE-79", "name": "XSS" },
                { "id": "CWE-80" }
            ],
            "metrics": [{
                "products": ["p1"],
                "content": {
                    "cvss_v3": { "baseScore": 6.1, "baseSeverity": "MEDIUM" },
                    "cvss_v4": { "baseScore": 7.0, "baseSeverity": "HIGH" }
                }
            }]
        }]
    }));

    assert_eq!(content.tlp.as_deref(), Some("CLEAR"));
    assert_eq!(
        content.publisher.unwrap().category.as_deref(),
        Some("coordinator")
    );
    let vuln = &content.vulnerabilities[0];
    assert_eq!(vuln.cwes.len(), 2);
    assert_eq!(vuln.score, Some(7.0));
    assert_eq!(vuln.severity.as_deref(), Some("HIGH"));
}

#[test]
fn minimal() {
    let content = extract(json!({ "document": { "title": "Minimal" } }));

    assert_eq!(content.tlp, None);
    assert_eq!(content.publisher, None);
    assert!(content.notes.is_empty());
    assert!(content.references.is_empty());
    assert!(content.vulnerabilities.is_empty());
}

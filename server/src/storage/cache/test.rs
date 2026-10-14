//! Tests for the cross-provider caches.

use super::CacheConfig;
use crate::{
    models::result::{
        DocumentProfileResults, DocumentValidation, ProfileResults, ProviderSummary,
        VulnerabilityEntry,
    },
    storage::Storage,
};
use std::time::Duration;
use tempfile::TempDir;
use time::OffsetDateTime;

/// Creates storage in a temporary directory with the given cache settings.
fn storage(config: CacheConfig) -> (TempDir, Storage) {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::with_cache(dir.path(), &config).unwrap();
    (dir, storage)
}

/// Builds a minimal summary for a provider.
pub(crate) fn summary(provider: &str, document_count: u64) -> ProviderSummary {
    ProviderSummary {
        checks: None,
        provider: provider.into(),
        publisher_name: None,
        validated_at: OffsetDateTime::now_utc(),
        document_count,
        profiles: ProfileResults {
            basic: None,
            extended: None,
            full: None,
        },
        signatures: None,
        top_failing_tests: vec![],
        retrieval_errors: 0,
        note: None,
    }
}

/// Builds a document referencing the given CVEs.
pub(crate) fn document(tracking_id: &str, released: &str, cves: &[&str]) -> DocumentValidation {
    DocumentValidation {
        checks: Default::default(),
        tracking_id: tracking_id.into(),
        title: format!("Advisory {tracking_id}"),
        url: format!("https://example.com/{tracking_id}.json"),
        profiles: DocumentProfileResults {
            basic: None,
            extended: None,
            full: None,
        },
        signature_error: None,
        signature_warning: None,
        signature_present: false,
        category: None,
        publisher_name: Some("Example".into()),
        initial_release_date: None,
        current_release_date: Some(released.into()),
        status: Some("final".into()),
        revision: None,
        aggregate_severity: None,
        csaf_version: Some("2.0".into()),
        revision_history: vec![],
        vulnerabilities: cves
            .iter()
            .map(|cve| VulnerabilityEntry {
                cve: Some((*cve).into()),
                ids: vec![],
                title: None,
                score: Some(7.5),
                severity: Some("HIGH".into()),
            })
            .collect(),
        version_count: 1,
        retrieval_error: None,
    }
}

/// Stores documents and a summary for a provider.
pub(crate) async fn provider(storage: &Storage, domain: &str, docs: &[DocumentValidation]) {
    storage.save_documents(domain, docs).await.unwrap();
    storage
        .save_summary(domain, &summary(domain, docs.len() as u64))
        .await
        .unwrap();
}

#[tokio::test]
async fn finds_cve_across_providers() {
    let (_dir, storage) = storage(CacheConfig::default());
    provider(
        &storage,
        "a.example",
        &[
            document("A-1", "2024-01-01T00:00:00Z", &["CVE-2024-0001"]),
            document("A-2", "2024-03-01T00:00:00Z", &["CVE-2024-0002"]),
        ],
    )
    .await;
    provider(
        &storage,
        "b.example",
        &[document(
            "B-1",
            "2024-02-01T00:00:00Z",
            &["CVE-2024-0001", "CVE-2023-9999"],
        )],
    )
    .await;

    let result = storage.find_by_cve("cve-2024-0001", false).await.unwrap();
    assert!(!result.truncated);
    let found: Vec<_> = result
        .hits
        .iter()
        .map(|h| (h.domain.as_str(), h.tracking_id.as_str()))
        .collect();
    assert_eq!(found, [("b.example", "B-1"), ("a.example", "A-1")]);
    assert_eq!(result.hits[0].score, Some(7.5));

    let result = storage.find_by_cve("CVE-2024-", true).await.unwrap();
    let mut cves: Vec<_> = result.hits.iter().map(|h| h.cve.as_str()).collect();
    cves.sort();
    assert_eq!(cves, ["CVE-2024-0001", "CVE-2024-0001", "CVE-2024-0002"]);

    assert!(
        storage
            .find_by_cve("CVE-2000-0000", false)
            .await
            .unwrap()
            .hits
            .is_empty()
    );
}

#[tokio::test]
async fn truncates_to_max_hits() {
    let (_dir, storage) = storage(CacheConfig {
        cve_search_max_hits: 2,
        ..Default::default()
    });
    let docs: Vec<_> = (0..3)
        .map(|i| {
            document(
                &format!("D-{i}"),
                "2024-01-01T00:00:00Z",
                &["CVE-2024-0001"],
            )
        })
        .collect();
    provider(&storage, "a.example", &docs).await;

    let result = storage.find_by_cve("CVE-2024-0001", false).await.unwrap();
    assert_eq!(result.hits.len(), 2);
    assert!(result.truncated);
}

#[tokio::test]
async fn caches_until_provider_job_finishes() {
    let (_dir, storage) = storage(CacheConfig::default());
    provider(
        &storage,
        "a.example",
        &[document("A-1", "2024-01-01T00:00:00Z", &["CVE-2024-0001"])],
    )
    .await;
    assert_eq!(
        storage
            .find_by_cve("CVE-2024-0001", false)
            .await
            .unwrap()
            .hits
            .len(),
        1
    );

    // Change the stored data without going through the invalidation path.
    storage
        .save_documents(
            "a.example",
            &[document("A-2", "2024-01-02T00:00:00Z", &["CVE-2024-0001"])],
        )
        .await
        .unwrap();
    let cached = storage.find_by_cve("CVE-2024-0001", false).await.unwrap();
    assert_eq!(cached.hits.len(), 1, "served from the cache");

    storage
        .invalidate_provider_caches(&["a.example".into()])
        .await
        .unwrap();
    let fresh = storage.find_by_cve("CVE-2024-0001", false).await.unwrap();
    assert_eq!(fresh.hits.len(), 2);
}

#[tokio::test]
async fn expires_after_ttl() {
    let (_dir, storage) = storage(CacheConfig {
        cve_search_ttl: Duration::from_millis(50),
        ..Default::default()
    });
    provider(
        &storage,
        "a.example",
        &[document("A-1", "2024-01-01T00:00:00Z", &["CVE-2024-0001"])],
    )
    .await;
    storage.find_by_cve("CVE-2024-0001", false).await.unwrap();
    storage
        .save_documents(
            "a.example",
            &[document("A-2", "2024-01-02T00:00:00Z", &["CVE-2024-0001"])],
        )
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;
    let result = storage.find_by_cve("CVE-2024-0001", false).await.unwrap();
    assert_eq!(result.hits.len(), 2);
}

#[tokio::test]
async fn evicts_beyond_capacity() {
    let (_dir, storage) = storage(CacheConfig {
        cve_search_capacity: 5,
        ..Default::default()
    });
    provider(&storage, "a.example", &[]).await;
    for i in 0..20 {
        storage
            .find_by_cve(&format!("CVE-2024-{i:04}"), false)
            .await
            .unwrap();
    }
    assert!(storage.cve_search.entry_count().await <= 5);
}

#[tokio::test]
async fn summary_index_tracks_writes_and_deletes() {
    let (_dir, storage) = storage(CacheConfig::default());
    provider(&storage, "a.example", &[]).await;
    provider(&storage, "b.example", &[]).await;

    let domains: Vec<_> = storage
        .list_summaries()
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.provider)
        .collect();
    assert_eq!(domains, ["a.example", "b.example"]);

    storage
        .save_summary("a.example", &summary("a.example", 42))
        .await
        .unwrap();
    let summaries = storage.list_summaries().await.unwrap();
    assert_eq!(summaries[0].document_count, 42);

    storage.delete_provider("b.example").await.unwrap();
    let domains: Vec<_> = storage
        .list_summaries()
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.provider)
        .collect();
    assert_eq!(domains, ["a.example"]);
}

#[tokio::test]
async fn reload_picks_up_external_summary_changes() {
    let (dir, storage) = storage(CacheConfig::default());
    provider(&storage, "a.example", &[]).await;
    assert_eq!(storage.list_summaries().await.unwrap()[0].document_count, 0);

    let path = dir.path().join("results/a.example/summary.json");
    let mut json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    json["document_count"] = 7.into();
    std::fs::write(&path, json.to_string()).unwrap();

    storage
        .invalidate_provider_caches(&["a.example".into()])
        .await
        .unwrap();
    assert_eq!(storage.list_summaries().await.unwrap()[0].document_count, 7);
}

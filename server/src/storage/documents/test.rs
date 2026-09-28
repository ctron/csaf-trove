//! Regression tests using real Git history and migrated SQLite databases.

#![cfg(test)]

use super::{ProviderInfo, load_provider_info, save_provider_info};
use csaf_trove_migration::{Migrator, MigratorTrait};
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement};

/// Opens a database and applies the requested prefix of migrations.
async fn database(url: &str, steps: Option<u32>) -> DatabaseConnection {
    let db = Database::connect(url).await.unwrap();
    Migrator::up(&db, steps).await.unwrap();
    db
}

/// Representative provider metadata with both aggregator flags covered.
fn provider_info() -> ProviderInfo {
    ProviderInfo {
        canonical_url: "https://example.com/provider-metadata.json".into(),
        publisher_name: "Example".into(),
        publisher_category: "vendor".into(),
        publisher_namespace: "https://example.com".into(),
        role: Some("csaf_provider".into()),
        list_on_aggregators: true,
        mirror_on_aggregators: false,
        last_updated: "2026-09-24T00:00:00Z".into(),
    }
}

/// Provider metadata can be inserted and replaced on a freshly migrated database.
#[tokio::test]
async fn provider_metadata_round_trip() {
    let db = database("sqlite::memory:", None).await;
    assert!(load_provider_info(&db).await.unwrap().is_none());
    let mut info = provider_info();
    save_provider_info(&db, &info).await.unwrap();
    info.publisher_name = "Updated publisher".into();
    info.list_on_aggregators = false;
    info.mirror_on_aggregators = true;
    save_provider_info(&db, &info).await.unwrap();
    let loaded = load_provider_info(&db).await.unwrap().unwrap();
    assert_eq!(loaded.publisher_name, info.publisher_name);
    assert_eq!(loaded.canonical_url, info.canonical_url);
    assert_eq!(loaded.publisher_category, info.publisher_category);
    assert_eq!(loaded.publisher_namespace, info.publisher_namespace);
    assert_eq!(loaded.role, info.role);
    assert_eq!(loaded.last_updated, info.last_updated);
    assert!(!loaded.list_on_aggregators);
    assert!(loaded.mirror_on_aggregators);
}

/// Existing misnamed tables are repaired with their stored metadata intact.
#[tokio::test]
async fn provider_metadata_upgrade_preserves_rows() {
    let db = database("sqlite::memory:", Some(4)).await;
    db.execute_unprepared(
        "INSERT INTO provider_info_table
         (id, canonical_url, publisher_name, publisher_category, publisher_namespace, last_updated)
         VALUES (1, 'https://example.com/meta.json', 'Existing', 'vendor', 'https://example.com', 'yesterday')",
    ).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    assert_eq!(
        load_provider_info(&db)
            .await
            .unwrap()
            .unwrap()
            .publisher_name,
        "Existing"
    );
    save_provider_info(&db, &provider_info()).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    assert_eq!(
        load_provider_info(&db)
            .await
            .unwrap()
            .unwrap()
            .publisher_name,
        "Example"
    );
    Migrator::down(&db, Some(1)).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    assert_eq!(
        load_provider_info(&db)
            .await
            .unwrap()
            .unwrap()
            .publisher_name,
        "Example"
    );
}

/// Legacy databases with the correct table keep their authoritative metadata.
#[tokio::test]
async fn provider_metadata_upgrade_preserves_legacy_table() {
    let db = database("sqlite::memory:", Some(4)).await;
    db.execute_unprepared("ALTER TABLE provider_info_table RENAME TO provider_info")
        .await
        .unwrap();
    save_provider_info(&db, &provider_info()).await.unwrap();
    db.execute_unprepared(
        "CREATE TABLE provider_info_table AS SELECT * FROM provider_info;
         UPDATE provider_info_table SET publisher_name = 'Obsolete copy'",
    )
    .await
    .unwrap();
    Migrator::up(&db, None).await.unwrap();
    assert_eq!(
        load_provider_info(&db)
            .await
            .unwrap()
            .unwrap()
            .publisher_name,
        "Example"
    );
    Migrator::down(&db, Some(1)).await.unwrap();
    assert_eq!(
        load_provider_info(&db)
            .await
            .unwrap()
            .unwrap()
            .publisher_name,
        "Example"
    );
}

/// Digest warnings survive persistence and are cleared when later checks pass.
#[tokio::test]
async fn digest_warning_round_trip() {
    let db = database("sqlite::memory:", None).await;
    db.execute_raw(Statement::from_string(
        DbBackend::Sqlite,
        "INSERT INTO documents (tracking_id, title, url, signature_present, version_count) VALUES ('example', 'Example', 'https://example.com/example.json', 1, 1)",
    ))
    .await
    .unwrap();
    let mut doc = super::load_document(&db, "example").await.unwrap().unwrap();
    assert!(doc.signature_warning.is_none());
    doc.signature_warning = Some("SHA-256 mismatch: expected <!doctype, got abc".into());
    super::save_documents(&db, &[doc.clone()]).await.unwrap();
    let loaded = super::load_document(&db, "example").await.unwrap().unwrap();
    assert_eq!(loaded.signature_warning, doc.signature_warning);
    assert!(loaded.signature_error.is_none());
    assert!(loaded.signature_present);
    doc.signature_warning = None;
    super::save_documents(&db, &[doc]).await.unwrap();
    assert!(
        super::load_document(&db, "example")
            .await
            .unwrap()
            .unwrap()
            .signature_warning
            .is_none()
    );
}

/// Passing and warnings-and-above partition all documents; infos alone still pass.
#[tokio::test]
async fn status_filters_partition_documents() {
    let db = database("sqlite::memory:", None).await;
    db.execute_unprepared(
        "INSERT INTO documents (tracking_id, title, url, signature_present, version_count,
            basic_passed, basic_error_count, extended_passed, extended_error_count,
            extended_warning_count, full_passed, full_error_count, full_info_count,
            signature_error, retrieval_error) VALUES
         ('clean', '', '', 1, 1, 1, 0, 1, 0, 0, 1, 0, 0, NULL, NULL),
         ('info', '', '', 1, 1, 1, 0, 1, 0, 0, 0, 0, 5, NULL, NULL),
         ('warning', '', '', 1, 1, 1, 0, 0, 0, 3, 0, 0, 2, NULL, NULL),
         ('invalid', '', '', 1, 1, 0, 1, 0, 1, 0, 0, 1, 0, NULL, NULL),
         ('signature', '', '', 1, 1, 1, 0, 1, 0, 0, 1, 0, 0, 'bad', NULL),
         ('digest', '', '', 1, 1, 1, 0, 1, 0, 0, 1, 0, 0, NULL, NULL),
         ('retrieval', '', '', 0, 1, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 'gone')",
    )
    .await
    .unwrap();
    db.execute_unprepared(
        "UPDATE documents SET signature_warning = 'mismatch' WHERE tracking_id = 'digest'",
    )
    .await
    .unwrap();
    let ids = async |status| {
        super::load_documents_paginated(&db, 0, 100, status)
            .await
            .unwrap()
            .items
            .into_iter()
            .map(|d| d.tracking_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(None).await.len(), 7);
    assert_eq!(ids(Some("passing")).await, ["clean", "info"]);
    assert_eq!(
        ids(Some("failing")).await,
        ["invalid", "retrieval", "signature"]
    );
    assert_eq!(
        ids(Some("warnings")).await,
        ["digest", "invalid", "retrieval", "signature", "warning"]
    );
    assert_eq!(ids(Some("errors")).await, ["retrieval"]);
}

/// Revalidation preserves revision counts and replaces obsolete tracking IDs at the same URL.
#[tokio::test]
async fn replacement_preserves_history_and_removes_stale_identity() {
    let db = database("sqlite::memory:", None).await;
    db.execute_unprepared("INSERT INTO documents (tracking_id, title, url, signature_present, version_count) VALUES ('old-id', 'Document', 'https://example.com/a.json', 0, 7)").await.unwrap();
    let mut doc = super::load_document(&db, "old-id").await.unwrap().unwrap();
    doc.tracking_id = "new-id".into();
    doc.version_count = 1;
    super::save_documents(&db, &[doc]).await.unwrap();
    assert!(super::load_document(&db, "old-id").await.unwrap().is_none());
    assert_eq!(
        super::load_document(&db, "new-id")
            .await
            .unwrap()
            .unwrap()
            .version_count,
        7
    );
    assert_eq!(super::document_count(&db).await.unwrap(), 1);
}

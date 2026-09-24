//! Regression tests using real Git history and migrated SQLite databases.

#![cfg(test)]

use super::{ProviderInfo, load_provider_info, save_provider_info, update_version_counts};
use crate::storage::git_repo::{commit_all, prepare_worktree};
use csaf_trove_entity::document;
use csaf_trove_migration::{Migrator, MigratorTrait};
use sea_orm::{
    ConnectionTrait, Database, DatabaseConnection, DbBackend, EntityTrait, QueryOrder, Statement,
};
use std::{fs, path::Path, time::Instant};

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

/// Records two versions of one document and one version of an untouched document.
fn history(repo: &Path, work: &Path) {
    let prepared = prepare_worktree(repo, work, false).unwrap();
    fs::create_dir_all(work.join("example.com")).unwrap();
    fs::write(work.join("example.com/a.json"), "first").unwrap();
    fs::write(work.join("example.com/b.json"), "unchanged").unwrap();
    commit_all(&prepared, "initial").unwrap();
    let prepared = prepare_worktree(repo, work, true).unwrap();
    fs::create_dir_all(work.join("example.com")).unwrap();
    fs::write(work.join("example.com/a.json"), "second").unwrap();
    commit_all(&prepared, "update").unwrap();
}

/// Counts duplicate URLs correctly, preserves missing history, and skips unchanged rows.
#[tokio::test]
async fn version_counts_update_only_changed_rows() {
    let db = database("sqlite::memory:", None).await;
    db.execute_unprepared(
        "INSERT INTO documents (id, tracking_id, title, url, signature_present, version_count) VALUES
         (1, 'a', 'a', 'https://example.com/a.json', 0, 1),
         (2, 'duplicate', 'duplicate', 'https://example.com/a.json', 0, 9),
         (3, 'b', 'b', 'https://example.com/b.json', 0, 1),
         (4, 'missing', 'missing', 'https://example.com/missing.json', 0, 7),
         (5, 'invalid', 'invalid', 'invalid URL', 0, 9);
         CREATE TABLE updates (id INTEGER);
         CREATE TRIGGER record_update AFTER UPDATE OF version_count ON documents
         BEGIN INSERT INTO updates VALUES (NEW.id); END;",
    ).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo.git");
    history(&repo, &dir.path().join("work"));
    update_version_counts(&db, &repo).await.unwrap();
    update_version_counts(&db, &repo).await.unwrap();
    let rows = document::Entity::find()
        .order_by_asc(document::Column::Id)
        .all(&db)
        .await
        .unwrap();
    assert_eq!(
        rows.iter().map(|row| row.version_count).collect::<Vec<_>>(),
        [2, 2, 1, 7, 9]
    );
    let updates = db
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT id FROM updates ORDER BY id",
        ))
        .await
        .unwrap();
    assert_eq!(
        updates.len(),
        2,
        "unchanged counts must not be written again"
    );
    assert_eq!(updates[0].try_get::<i64>("", "id").unwrap(), 1);
    assert_eq!(updates[1].try_get::<i64>("", "id").unwrap(), 2);
}

/// Missing Git repositories and empty document tables do not alter stored counts.
#[tokio::test]
async fn version_counts_handle_empty_inputs() {
    let db = database("sqlite::memory:", None).await;
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo.git");
    update_version_counts(&db, &repo).await.unwrap();
    history(&repo, &dir.path().join("work"));
    update_version_counts(&db, &repo).await.unwrap();
}

/// Measures the database update path at SUSE's document count, without a URL index.
#[tokio::test]
#[ignore = "manual performance check with 111427 rows"]
async fn benchmark_version_count_updates() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("documents.db");
    let db = database(&format!("sqlite://{}?mode=rwc", db_path.display()), None).await;
    db.execute_unprepared(
        "WITH RECURSIVE ids(n) AS (SELECT 1 UNION ALL SELECT n+1 FROM ids WHERE n<111427)
         INSERT INTO documents (id, tracking_id, title, url, signature_present, version_count)
         SELECT n, CAST(n AS TEXT), 'Document', 'https://example.com/a.json', 0, 1 FROM ids",
    )
    .await
    .unwrap();
    let repo = dir.path().join("repo.git");
    history(&repo, &dir.path().join("work"));
    let started = Instant::now();
    update_version_counts(&db, &repo).await.unwrap();
    eprintln!("111427 changed rows: {:?}", started.elapsed());
    let started = Instant::now();
    update_version_counts(&db, &repo).await.unwrap();
    eprintln!("111427 unchanged rows: {:?}", started.elapsed());
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

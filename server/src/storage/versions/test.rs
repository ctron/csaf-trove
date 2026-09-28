//! Version recording tests using real Git history and migrated SQLite databases.

#![cfg(test)]

use super::{list_versions, record_versions, versions_missing};
use crate::{
    models::result::DiffTag,
    storage::{
        Storage,
        git_repo::{commit_all, prepare_worktree},
    },
};
use csaf_trove_entity::document;
use csaf_trove_migration::{Migrator, MigratorTrait};
use sea_orm::{
    ConnectionTrait, Database, DatabaseConnection, DbBackend, EntityTrait, QueryOrder, Statement,
};
use std::{fs, path::Path, time::Instant};

/// Opens a database and applies all migrations.
async fn database(url: &str) -> DatabaseConnection {
    let db = Database::connect(url).await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    db
}

/// Reads one scalar from a test query.
async fn scalar(db: &DatabaseConnection, sql: &str) -> i64 {
    db.query_one_raw(Statement::from_string(DbBackend::Sqlite, sql))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "value")
        .unwrap()
}

/// Returns the current HEAD commit of a bare repository.
fn head(repo: &Path) -> String {
    git2::Repository::open_bare(repo)
        .unwrap()
        .head()
        .unwrap()
        .target()
        .unwrap()
        .to_string()
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
    let db = database("sqlite::memory:").await;
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
    record_versions(&db, &repo, None).await.unwrap();
    record_versions(&db, &repo, None).await.unwrap();
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
    let db = database("sqlite::memory:").await;
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo.git");
    record_versions(&db, &repo, None).await.unwrap();
    history(&repo, &dir.path().join("work"));
    record_versions(&db, &repo, None).await.unwrap();
}

/// Measures the database update path at SUSE's document count, without a URL index.
#[tokio::test]
#[ignore = "manual performance check with 111427 rows"]
async fn benchmark_version_count_updates() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("documents.db");
    let db = database(&format!("sqlite://{}?mode=rwc", db_path.display())).await;
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
    record_versions(&db, &repo, None).await.unwrap();
    eprintln!("111427 changed rows: {:?}", started.elapsed());
    let started = Instant::now();
    record_versions(&db, &repo, None).await.unwrap();
    eprintln!("111427 unchanged rows: {:?}", started.elapsed());
}

/// Incremental passes append only new versions, and retries never duplicate them.
#[tokio::test]
async fn incremental_recording_is_idempotent_and_paginated() {
    let db = database("sqlite::memory:").await;
    db.execute_unprepared(
        "INSERT INTO documents (id, tracking_id, title, url, signature_present, version_count) VALUES
         (1, 'a', 'a', 'https://example.com/a.json', 0, 1)",
    )
    .await
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo.git");
    let work = dir.path().join("work");
    assert!(versions_missing(&db).await.unwrap());
    history(&repo, &work);
    record_versions(&db, &repo, None).await.unwrap();
    assert!(!versions_missing(&db).await.unwrap());
    let checkpoint = head(&repo);

    let prepared = prepare_worktree(&repo, &work, true).unwrap();
    fs::create_dir_all(work.join("example.com")).unwrap();
    fs::write(work.join("example.com/a.json"), "third").unwrap();
    commit_all(&prepared, "third").unwrap();
    record_versions(&db, &repo, Some(&checkpoint))
        .await
        .unwrap();
    record_versions(&db, &repo, Some(&checkpoint))
        .await
        .unwrap();
    assert_eq!(
        scalar(&db, "SELECT COUNT(*) AS value FROM document_versions").await,
        4
    );
    assert_eq!(
        scalar(
            &db,
            "SELECT version_count AS value FROM documents WHERE id = 1"
        )
        .await,
        3
    );

    let url = "https://example.com/a.json";
    let first = list_versions(&db, url, 0, 2).await.unwrap();
    assert_eq!(first.total, 3);
    assert_eq!(
        first
            .items
            .iter()
            .map(|v| v.message.as_str())
            .collect::<Vec<_>>(),
        ["third", "update"]
    );
    assert!(first.items[0].is_latest);
    assert!(!first.items[1].is_latest);
    let second = list_versions(&db, url, 2, 2).await.unwrap();
    assert_eq!(second.items.len(), 1);
    assert_eq!(second.items[0].message, "initial");
    assert!(!second.items[0].is_latest);
}

/// Historical reads and diffs resolve blobs from recorded versions beyond any page size.
#[tokio::test]
async fn storage_reads_versions_by_blob() {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::new(dir.path()).unwrap();
    let domain = "example.com";
    let repo = storage.repo_path(domain);
    let work = dir.path().join("work");
    let doc = |n: usize| {
        serde_json::to_vec(
            &serde_json::json!({"document": {"title": format!("v{n}"), "tracking": {
                "id": "a", "status": "final", "version": n.to_string()
            }}}),
        )
        .unwrap()
    };
    for n in 1..=60 {
        let prepared = prepare_worktree(&repo, &work, n > 1).unwrap();
        fs::create_dir_all(work.join("example.com")).unwrap();
        fs::write(work.join("example.com/a.json"), doc(n)).unwrap();
        commit_all(&prepared, &format!("v{n}")).unwrap();
    }
    let db = storage.db.get(domain).await.unwrap();
    db.execute_unprepared(
        "INSERT INTO documents (tracking_id, title, url, signature_present, version_count) VALUES
         ('a', 'a', 'https://example.com/a.json', 0, 1)",
    )
    .await
    .unwrap();
    storage.record_versions(domain, None).await.unwrap();

    let page = storage
        .document_versions(domain, "a", 55, 10)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(page.total, 60);
    assert_eq!(page.items.len(), 5);
    let oldest = page.items.last().unwrap();
    assert_eq!(oldest.version.as_deref(), Some("1"));
    assert_eq!(oldest.status.as_deref(), Some("final"));

    let historical = storage
        .read_historical_document(domain, "a", &oldest.commit_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(historical.title, "v1");
    let diff = storage
        .diff_document_versions(domain, "a", &oldest.commit_id)
        .await
        .unwrap()
        .unwrap();
    assert!(
        diff.iter()
            .any(|l| matches!(l.tag, DiffTag::Insert) && l.content.contains("v2"))
    );
    let latest = storage
        .document_versions(domain, "a", 0, 1)
        .await
        .unwrap()
        .unwrap();
    assert!(
        storage
            .diff_document_versions(domain, "a", &latest.items[0].commit_id)
            .await
            .unwrap()
            .is_none()
    );
}

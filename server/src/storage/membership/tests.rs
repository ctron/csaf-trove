//! Distribution health with actual Git metadata and a migrated provider database.

use super::*;
use crate::storage::{
    documents::distribution_health,
    git_repo::{commit_snapshot, prepare_worktree},
};
use sea_orm::DatabaseConnection;
use std::{fs, path::Path};
use tempfile::TempDir;

/// Two feeds sharing a parent directory must never share inferred membership.
const WHITE: &str = "https://feeds.example/.well-known/csaf/white.json";
/// Second feed used for independent and overlapping membership assertions.
const GREEN: &str = "https://feeds.example/.well-known/csaf/green.json";
/// Directory whose index points outside its own URL prefix.
const DIRECTORY: &str = "https://feeds.example/advisories/";
/// Advisory stored on a different host from its feed.
const DOCUMENT: &str = "https://cdn.example/csaf/white/2026/advisory.json";
/// An unrelated advisory under the feed's parent path.
const UNRELATED: &str = "https://feeds.example/.well-known/csaf/unrelated.json";

/// Creates persisted metadata, one validated advisory and one retrieval failure.
async fn fixture() -> (TempDir, Storage, DatabaseConnection) {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::new(dir.path()).unwrap();
    let work = dir.path().join("work");
    let prepared = prepare_worktree(&storage.repo_path("example.com"), &work, false).unwrap();
    fs::create_dir_all(work.join("metadata")).unwrap();
    fs::write(
        work.join("metadata/provider-metadata.json"),
        serde_json::to_vec(&serde_json::json!({
            "distributions": [{"directory_url": DIRECTORY, "rolie":{"feeds":[
                {"url": WHITE, "tlp_label":"CLEAR"}, {"url":GREEN, "tlp_label":"GREEN"}
            ]}}]
        }))
        .unwrap(),
    )
    .unwrap();
    commit_snapshot(&prepared, "metadata").unwrap();
    storage
        .save_retrieval_errors(
            "example.com",
            &[
                (DOCUMENT.into(), "temporary".into()),
                (UNRELATED.into(), "HTTP 404".into()),
            ],
        )
        .await
        .unwrap();
    let db = storage.db.get("example.com").await.unwrap();
    db.execute_raw(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "UPDATE documents SET retrieval_error = NULL,
            basic_test_count = 4, basic_failing_test_count = 1,
            extended_test_count = 5, extended_failing_test_count = 1,
            full_test_count = 10, full_failing_test_count = 1 WHERE url = ?",
        [DOCUMENT.into()],
    ))
    .await
    .unwrap();
    (dir, storage, db)
}

/// Builds complete index snapshots using the same mapping for feeds and directories.
fn membership(entries: &[(&str, &[&str])]) -> DistributionMembership {
    entries
        .iter()
        .map(|(distribution, urls)| {
            (
                distribution.to_string(),
                urls.iter().map(|url| url.to_string()).collect(),
            )
        })
        .collect()
}

/// Counts and all profile rates follow actual membership, not URL prefixes.
#[tokio::test]
async fn health_uses_exact_membership_and_survives_unchanged_syncs() {
    let (dir, storage, db) = fixture().await;
    let indexes = membership(&[
        (WHITE, &[DOCUMENT, DOCUMENT]),
        (GREEN, &[UNRELATED]),
        (DIRECTORY, &[DOCUMENT]),
    ]);
    storage
        .save_distribution_membership("example.com", &indexes)
        .await
        .unwrap();
    let health = storage
        .compute_distribution_health("example.com", &[DIRECTORY.into()])
        .await
        .unwrap();
    assert_eq!(health.len(), 3);
    let white = health.iter().find(|h| h.url == WHITE).unwrap();
    assert_eq!(white.document_count, 1);
    assert_eq!(white.retrieval_errors, 0);
    assert_eq!(white.basic_pass_rate, Some(0.75));
    assert_eq!(white.extended_pass_rate, Some(0.8));
    assert_eq!(white.full_pass_rate, Some(0.9));
    let green = health.iter().find(|h| h.url == GREEN).unwrap();
    assert_eq!((green.document_count, green.retrieval_errors), (1, 1));
    assert_eq!(green.basic_pass_rate, None);
    assert!(health.iter().find(|h| h.url == DIRECTORY).unwrap().skipped);

    // An unchanged sync must not write existing membership rows.
    db.execute_unprepared("CREATE TRIGGER reject_membership_insert BEFORE INSERT ON distribution_membership BEGIN SELECT RAISE(ABORT, 'unexpected rewrite'); END;
        CREATE TRIGGER reject_membership_delete BEFORE DELETE ON distribution_membership BEGIN SELECT RAISE(ABORT, 'unexpected rewrite'); END;").await.unwrap();
    storage
        .save_distribution_membership("example.com", &indexes)
        .await
        .unwrap();
    db.execute_unprepared(
        "DROP TRIGGER reject_membership_insert; DROP TRIGGER reject_membership_delete;",
    )
    .await
    .unwrap();

    // Persistence survives reopening and does not depend on a new Git commit or validation pass.
    let reopened = Storage::new(Path::new(dir.path())).unwrap();
    assert_eq!(
        reopened
            .compute_distribution_health("example.com", &[])
            .await
            .unwrap()[1]
            .document_count,
        1
    );
    // Missing (failed/skipped) indexes retain their previous members; a successful empty one clears.
    reopened
        .save_distribution_membership("example.com", &membership(&[(WHITE, &[])]))
        .await
        .unwrap();
    assert_eq!(distribution_health(&db, WHITE).await.unwrap().0, 0);
    assert_eq!(distribution_health(&db, GREEN).await.unwrap().0, 1);
    assert_eq!(distribution_health(&db, DIRECTORY).await.unwrap().0, 1);
    assert_eq!(storage.document_count("example.com").await.unwrap(), 2);
}

/// Large indexes are batched and partially failed updates roll back as a unit.
#[tokio::test]
async fn membership_batches_and_rolls_back_failed_updates() {
    let (_dir, storage, db) = fixture().await;
    let urls: BTreeSet<_> = (0..901)
        .map(|i| format!("https://cdn.example/{i}.json"))
        .collect();
    storage
        .save_distribution_membership(
            "example.com",
            &DistributionMembership::from([(WHITE.into(), urls)]),
        )
        .await
        .unwrap();
    let row = db
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT COUNT(*) AS count FROM distribution_membership",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<i64>("", "count").unwrap(), 901);
    db.execute_unprepared("CREATE TRIGGER fail_membership BEFORE INSERT ON distribution_membership BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").await.unwrap();
    assert!(
        storage
            .save_distribution_membership("example.com", &membership(&[(WHITE, &[DOCUMENT])]))
            .await
            .is_err()
    );
    let row = db
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT COUNT(*) AS count FROM distribution_membership",
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<i64>("", "count").unwrap(), 901);
    db.execute_unprepared("DROP TRIGGER fail_membership")
        .await
        .unwrap();
    storage
        .save_distribution_membership("example.com", &membership(&[(WHITE, &[DOCUMENT])]))
        .await
        .unwrap();
    assert_eq!(distribution_health(&db, WHITE).await.unwrap().0, 1);
}

//! End-to-end processing tests with real Git snapshots and SQLite results.
use super::*;
use crate::{
    Config,
    storage::{Storage, scratch},
};
use sea_orm::{ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement};
use std::{collections::HashMap, fs};
use tempfile::TempDir;
use tokio::sync::{Notify, RwLock, watch};

/// Creates isolated application state and a provider with overlapping distributions.
fn fixture() -> (TempDir, Arc<AppState>, Source) {
    let dir = tempfile::tempdir().unwrap();
    let config: Config = serde_json::from_value(serde_json::json!({
        "server": {"listen":"127.0.0.1:0"}, "data":{"dir": dir.path()}, "scheduler": {}
    }))
    .unwrap();
    let source: Source =
        serde_json::from_value(serde_json::json!({"domain":"example.com"})).unwrap();
    let state = Arc::new(AppState {
        config,
        storage: Storage::new(dir.path()).unwrap(),
        sources: RwLock::new(HashMap::new()),
        jobs: RwLock::new(HashMap::new()),
        api_token: None,
        webhook_secret: None,
        data_dir: dir.path().to_path_buf(),
        pipeline_locks: RwLock::new(HashMap::new()),
        sources_changed: Notify::new(),
        job_notify: watch::channel(()).0,
        recent_sync_points: RwLock::new(HashMap::new()),
    });
    let metadata = serde_json::to_vec(&serde_json::json!({
        "canonical_url":"https://example.com/provider-metadata.json", "metadata_version":"2.0",
        "last_updated":"2026-09-24T00:00:00Z", "publisher":{"name":"Example", "category":"vendor", "namespace":"https://example.com"},
        "distributions":[{"directory_url":"https://example.com/"},{"directory_url":"https://example.com/"}]
    })).unwrap();
    commit(
        &state,
        &[
            ("metadata/provider-metadata.json", &metadata),
            ("example.com/a.json", b"first a"),
            ("example.com/b.json", b"first b"),
        ],
    );
    (dir, state, source)
}

/// Publishes a download batch while preserving files outside it.
fn commit(state: &AppState, files: &[(&str, &[u8])]) {
    let repo = state.storage.repo_path("example.com");
    let work = state.work_dir().join("example.com");
    let prepared = git_repo::prepare_worktree(&repo, &work).unwrap();
    for (path, data) in files {
        scratch::write(&work, Path::new(path), data).unwrap();
    }
    git_repo::commit_snapshot(&prepared, "test").unwrap();
}

/// Runs local processing without provider networking.
async fn process(state: &Arc<AppState>, source: &Source) -> Result<()> {
    process_snapshot(
        state,
        source,
        &state.work_dir().join("example.com"),
        false,
        &[],
    )
    .await
}

/// Opens a second connection for failure injection and assertions.
async fn database(state: &AppState) -> DatabaseConnection {
    Database::connect(format!(
        "sqlite://{}",
        state
            .data_dir
            .join("results/example.com/documents.db")
            .display()
    ))
    .await
    .unwrap()
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

/// Unchanged and identical downloads do not rewrite results, counts or validation timestamps.
#[tokio::test]
async fn unchanged_processing_reuses_results_and_summary() {
    let (_dir, state, source) = fixture();
    process(&state, &source).await.unwrap();
    assert_eq!(
        state.storage.document_count(&source.domain).await.unwrap(),
        2
    );
    let summary = state
        .storage
        .load_summary(&source.domain)
        .await
        .unwrap()
        .unwrap();
    let db = database(&state).await;
    db.execute_unprepared("CREATE TRIGGER prohibit_insert BEFORE INSERT ON documents BEGIN SELECT RAISE(ABORT, 'unexpected document insert'); END;
        CREATE TRIGGER prohibit_update BEFORE UPDATE ON documents BEGIN SELECT RAISE(ABORT, 'unexpected document update'); END;
        CREATE TRIGGER prohibit_delete BEFORE DELETE ON documents BEGIN SELECT RAISE(ABORT, 'unexpected document delete'); END;").await.unwrap();
    commit(&state, &[("example.com/a.json", b"first a")]);
    process(&state, &source).await.unwrap();
    let after = state
        .storage
        .load_summary(&source.domain)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.validated_at, summary.validated_at);
    assert_eq!(
        scalar(&db, "SELECT SUM(version_count) AS value FROM documents").await,
        2
    );
    // A missing cached summary is rebuilt without touching document rows.
    fs::remove_file(state.data_dir.join("results/example.com/summary.json")).unwrap();
    process(&state, &source).await.unwrap();
    assert!(
        state
            .storage
            .load_summary(&source.domain)
            .await
            .unwrap()
            .is_some()
    );
}

/// Only changed documents are replaced; sidecar-only updates preserve version counts.
#[tokio::test]
async fn targeted_processing_preserves_untouched_rows() {
    let (_dir, state, source) = fixture();
    process(&state, &source).await.unwrap();
    let db = database(&state).await;
    db.execute_unprepared("CREATE TRIGGER protect_b BEFORE DELETE ON documents WHEN OLD.tracking_id = 'b' BEGIN SELECT RAISE(ABORT, 'untouched document deleted'); END;
        CREATE TRIGGER protect_b_update BEFORE UPDATE ON documents WHEN OLD.tracking_id = 'b' BEGIN SELECT RAISE(ABORT, 'untouched document updated'); END;").await.unwrap();
    commit(&state, &[("example.com/a.json", b"second a")]);
    process(&state, &source).await.unwrap();
    assert_eq!(
        scalar(
            &db,
            "SELECT version_count AS value FROM documents WHERE tracking_id = 'a'"
        )
        .await,
        2
    );
    commit(&state, &[("example.com/a.json.sha256", b"incorrect")]);
    process(&state, &source).await.unwrap();
    assert_eq!(
        scalar(
            &db,
            "SELECT version_count AS value FROM documents WHERE tracking_id = 'a'"
        )
        .await,
        2
    );
    assert_eq!(
        scalar(&db, "SELECT COUNT(*) AS value FROM documents").await,
        2
    );
}

/// Failure after validation does not publish a checkpoint; a later revert still retries.
#[tokio::test]
async fn interrupted_processing_replays_reverted_changes() {
    let (_dir, state, source) = fixture();
    process(&state, &source).await.unwrap();
    let checkpoint = state
        .storage
        .processing_checkpoint(&source.domain)
        .await
        .unwrap()
        .unwrap();
    let db = database(&state).await;
    db.execute_unprepared("CREATE TRIGGER fail_counts BEFORE UPDATE OF version_count ON documents BEGIN SELECT RAISE(ABORT, 'injected history failure'); END;").await.unwrap();
    commit(&state, &[("example.com/a.json", b"second a")]);
    assert!(process(&state, &source).await.is_err());
    assert_eq!(
        state
            .storage
            .processing_checkpoint(&source.domain)
            .await
            .unwrap()
            .unwrap()
            .commit_id,
        checkpoint.commit_id
    );
    commit(&state, &[("example.com/a.json", b"first a")]);
    db.execute_unprepared("DROP TRIGGER fail_counts")
        .await
        .unwrap();
    process(&state, &source).await.unwrap();
    assert_eq!(
        scalar(
            &db,
            "SELECT version_count AS value FROM documents WHERE tracking_id = 'a'"
        )
        .await,
        3
    );
    process(&state, &source).await.unwrap();
    assert_eq!(
        scalar(
            &db,
            "SELECT version_count AS value FROM documents WHERE tracking_id = 'a'"
        )
        .await,
        3
    );
}

/// Signature policy changes and explicit revalidation process every stored document.
#[tokio::test]
async fn policy_change_and_manual_revalidation_refresh_all() {
    let (_dir, state, mut source) = fixture();
    process(&state, &source).await.unwrap();
    let db = database(&state).await;
    db.execute_unprepared("CREATE TABLE writes (id INTEGER); CREATE TRIGGER record_inserts AFTER INSERT ON documents BEGIN INSERT INTO writes VALUES (NEW.id); END;").await.unwrap();
    source.accept_v3_signatures = true;
    process(&state, &source).await.unwrap();
    assert_eq!(scalar(&db, "SELECT COUNT(*) AS value FROM writes").await, 2);
    process_snapshot(
        &state,
        &source,
        &state.work_dir().join("example.com"),
        true,
        &[],
    )
    .await
    .unwrap();
    assert_eq!(scalar(&db, "SELECT COUNT(*) AS value FROM writes").await, 4);
    assert_eq!(
        scalar(&db, "SELECT SUM(version_count) AS value FROM documents").await,
        2
    );
}

/// A failure after changing retrieval status leaves summary work durable, even at the same HEAD.
#[tokio::test]
async fn retrieval_error_summary_recovery_and_identical_download() {
    let (_dir, state, source) = fixture();
    process(&state, &source).await.unwrap();
    // An empty new download batch avoids pretending the failed URL was retrieved successfully.
    commit(&state, &[]);
    let work = state.work_dir().join("example.com");
    let errors = vec![crate::pipeline::sync::RetrievalFailure {
        url: "https://example.com/a.json".into(),
        error: "HTTP 503".into(),
    }];
    let summary_path = state.data_dir.join("results/example.com/summary.json");
    let summary_bytes = fs::read(&summary_path).unwrap();
    fs::remove_file(&summary_path).unwrap();
    fs::create_dir(&summary_path).unwrap();
    assert!(
        process_snapshot(&state, &source, &work, false, &errors)
            .await
            .is_err()
    );
    assert!(
        state
            .storage
            .processing_checkpoint(&source.domain)
            .await
            .unwrap()
            .unwrap()
            .summary_dirty
    );
    fs::remove_dir(&summary_path).unwrap();
    fs::write(&summary_path, summary_bytes).unwrap();
    process_snapshot(&state, &source, &work, false, &errors)
        .await
        .unwrap();
    let summary = state
        .storage
        .load_summary(&source.domain)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.retrieval_errors, 1);
    // Repeated identical errors reuse the existing summary rather than rewriting it.
    process_snapshot(&state, &source, &work, false, &errors)
        .await
        .unwrap();
    assert_eq!(
        state
            .storage
            .load_summary(&source.domain)
            .await
            .unwrap()
            .unwrap()
            .validated_at,
        summary.validated_at
    );
    // The same advisory bytes must still clear a persisted retrieval error on recovery.
    commit(&state, &[("example.com/a.json", b"first a")]);
    process(&state, &source).await.unwrap();
    assert_eq!(
        state
            .storage
            .load_summary(&source.domain)
            .await
            .unwrap()
            .unwrap()
            .retrieval_errors,
        0
    );
    let db = database(&state).await;
    assert_eq!(
        scalar(
            &db,
            "SELECT version_count AS value FROM documents WHERE tracking_id = 'a'"
        )
        .await,
        1
    );
}

/// Interrupted manual revalidation invalidates its checkpoint and is repaired by the next sync.
#[tokio::test]
async fn interrupted_manual_revalidation_forces_recovery() {
    let (_dir, state, source) = fixture();
    process(&state, &source).await.unwrap();
    let db = database(&state).await;
    db.execute_unprepared("CREATE TRIGGER fail_validation BEFORE INSERT ON documents BEGIN SELECT RAISE(ABORT, 'injected validation failure'); END;").await.unwrap();
    assert!(
        process_snapshot(
            &state,
            &source,
            &state.work_dir().join("example.com"),
            true,
            &[]
        )
        .await
        .is_err()
    );
    assert!(
        state
            .storage
            .processing_checkpoint(&source.domain)
            .await
            .unwrap()
            .is_none()
    );
    db.execute_unprepared("DROP TRIGGER fail_validation")
        .await
        .unwrap();
    process(&state, &source).await.unwrap();
    assert_eq!(
        state.storage.document_count(&source.domain).await.unwrap(),
        2
    );
    assert!(
        state
            .storage
            .processing_checkpoint(&source.domain)
            .await
            .unwrap()
            .is_some()
    );
}

/// Existing installs backfill version history once, then record only new versions.
#[tokio::test]
async fn processing_backfills_and_extends_versions() {
    let (_dir, state, source) = fixture();
    process(&state, &source).await.unwrap();
    let db = database(&state).await;
    let versions = "SELECT COUNT(*) AS value FROM document_versions";
    assert_eq!(scalar(&db, versions).await, 2);
    // Simulate an install upgraded from before versions were recorded.
    db.execute_unprepared("DELETE FROM document_versions")
        .await
        .unwrap();
    process(&state, &source).await.unwrap();
    assert_eq!(scalar(&db, versions).await, 2);
    commit(&state, &[("example.com/a.json", b"second a")]);
    process(&state, &source).await.unwrap();
    assert_eq!(scalar(&db, versions).await, 3);
    assert_eq!(
        scalar(&db, "SELECT SUM(version_count) AS value FROM documents").await,
        3
    );
}

/// Real validation distinguishes parsing, OpenPGP authenticity, and mixed digest outcomes.
#[tokio::test]
async fn essential_checks_remain_independent_through_processing() {
    use csaf_trove_common::document_checks::CheckStatus;
    use sha2::{Digest, Sha256, Sha512};

    let (_dir, state, source) = fixture();
    let advisory = serde_json::to_vec(&serde_json::json!({
        "document": {
            "category": "csaf_base", "csaf_version": "2.0", "title": "Example",
            "publisher": {"category": "vendor", "name": "Example", "namespace": "https://example.com"},
            "tracking": {
                "id": "a", "status": "final", "version": "1",
                "initial_release_date": "2026-01-01T00:00:00Z",
                "current_release_date": "2026-01-01T00:00:00Z",
                "revision_history": [{"date": "2026-01-01T00:00:00Z", "number": "1", "summary": "Initial"}]
            }
        }
    })).unwrap();
    let sha256 = hex::encode(Sha256::digest(&advisory));
    let sha512 = hex::encode(Sha512::digest(&advisory));
    commit(
        &state,
        &[
            ("example.com/a.json", &advisory),
            ("example.com/a.json.sha256", sha256.as_bytes()),
            ("example.com/a.json.sha512", sha512.as_bytes()),
        ],
    );
    process(&state, &source).await.unwrap();
    let load = async || {
        state
            .storage
            .load_document(&source.domain, "a")
            .await
            .unwrap()
            .unwrap()
    };
    let checks = load().await.checks;
    assert_eq!(
        checks.parsing.status,
        CheckStatus::Passed,
        "{:?}",
        checks.parsing
    );
    assert_eq!(checks.signature.status, CheckStatus::Missing);
    assert_eq!(checks.digest.status, CheckStatus::Passed);

    commit(
        &state,
        &[("example.com/a.json.asc", b"not an OpenPGP signature")],
    );
    process(&state, &source).await.unwrap();
    let checks = load().await.checks;
    assert_eq!(checks.signature.status, CheckStatus::Failed);
    assert_eq!(checks.digest.status, CheckStatus::Passed);

    commit(&state, &[("example.com/a.json.sha512", b"incorrect")]);
    process(&state, &source).await.unwrap();
    let checks = load().await.checks;
    assert_eq!(checks.signature.status, CheckStatus::Failed);
    assert_eq!(checks.digest.status, CheckStatus::Warning);
    assert!(checks.digest.message.unwrap().contains("SHA-512 mismatch"));

    commit(&state, &[("example.com/a.json.sha256", b"incorrect")]);
    process(&state, &source).await.unwrap();
    let checks = load().await.checks;
    assert_eq!(checks.digest.status, CheckStatus::Failed);
    assert_eq!(checks.signature.status, CheckStatus::Failed);
    let summary = state
        .storage
        .load_summary(&source.domain)
        .await
        .unwrap()
        .unwrap();
    let counts = summary.checks.unwrap();
    assert_eq!(counts.signature.failed, 1);
    assert_eq!(counts.digest.failed, 1);
    assert_eq!(counts.parsing.failed, 1); // The fixture's other document is unparsable.
    let metrics = state.storage.load_metrics(&source.domain).await.unwrap();
    assert_eq!(
        metrics.entries.last().unwrap().checks.as_ref(),
        Some(&counts)
    );

    commit(&state, &[("example.com/a.json", b"not JSON")]);
    process(&state, &source).await.unwrap();
    let doc = load().await;
    assert_eq!(doc.checks.retrieval.status, CheckStatus::Passed);
    assert_eq!(doc.checks.parsing.status, CheckStatus::Failed);
    assert_eq!(doc.checks.signature.status, CheckStatus::NotEvaluated);
    assert_eq!(doc.checks.digest.status, CheckStatus::NotEvaluated);
    assert!(doc.profiles.basic.is_none());
}

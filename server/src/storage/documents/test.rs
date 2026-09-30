//! Regression tests using real Git history and migrated SQLite databases.

#![cfg(test)]

use super::{ProviderInfo, load_provider_info, save_provider_info};
use csaf_trove_common::document_checks::{CheckDetail, CheckOutcome, CheckStatus, DocumentChecks};
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
         ('parse', '', '', 0, 1, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 'Document error: invalid JSON', NULL),
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
    for (id, checks) in [
        ("clean", successful_checks()),
        ("info", successful_checks()),
        ("warning", successful_checks()),
        ("invalid", successful_checks()),
        (
            "parse",
            DocumentChecks {
                parsing: CheckOutcome::failed("invalid JSON"),
                signature: CheckOutcome::default(),
                digest: CheckOutcome::default(),
                ..successful_checks()
            },
        ),
        (
            "signature",
            DocumentChecks {
                signature: CheckOutcome::failed("bad signature"),
                ..successful_checks()
            },
        ),
        (
            "digest",
            DocumentChecks {
                digest: CheckOutcome {
                    status: CheckStatus::Warning,
                    message: Some("mismatch".into()),
                    ..Default::default()
                },
                ..successful_checks()
            },
        ),
        (
            "retrieval",
            DocumentChecks {
                retrieval: CheckOutcome::failed("gone"),
                ..Default::default()
            },
        ),
    ] {
        set_checks(&db, id, &checks).await;
    }
    let ids = async |status| {
        super::load_documents_paginated(&db, 0, 100, status)
            .await
            .unwrap()
            .items
            .into_iter()
            .map(|d| d.tracking_id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(None).await.len(), 8);
    assert_eq!(ids(Some("passing")).await, ["clean", "info"]);
    assert_eq!(
        ids(Some("failing")).await,
        ["invalid", "parse", "retrieval", "signature"]
    );
    assert_eq!(
        ids(Some("warnings")).await,
        [
            "digest",
            "invalid",
            "parse",
            "retrieval",
            "signature",
            "warning"
        ]
    );
    assert_eq!(ids(Some("errors")).await, ["retrieval"]);
    assert_eq!(ids(Some("signature-errors")).await, ["signature"]);
    assert_eq!(ids(Some("parse-errors")).await, ["parse"]);
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

/// Creates independent successful outcomes for a validated fixture.
fn successful_checks() -> DocumentChecks {
    DocumentChecks {
        retrieval: CheckOutcome::new(CheckStatus::Passed),
        parsing: CheckOutcome::new(CheckStatus::Passed),
        signature: CheckOutcome::new(CheckStatus::Passed),
        digest: CheckOutcome::new(CheckStatus::Passed),
    }
}

/// Stores explicit outcomes for a fixture without inferring them from legacy fields.
async fn set_checks(db: &DatabaseConnection, id: &str, checks: &DocumentChecks) {
    db.execute_raw(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "UPDATE documents SET document_checks = ? WHERE tracking_id = ?",
        [serde_json::to_string(checks).unwrap().into(), id.into()],
    ))
    .await
    .unwrap();
}

/// Migration preserves known failures without guessing whether old combined successes were signed.
#[tokio::test]
async fn legacy_check_migration_is_conservative() {
    let db = database("sqlite::memory:", Some(10)).await;
    db.execute_unprepared("INSERT INTO documents (tracking_id, title, url, signature_present, basic_passed, signature_error, signature_warning, retrieval_error) VALUES
        ('parsed', '', '', 1, 1, NULL, NULL, NULL),
        ('parse', '', '', 0, NULL, 'Document error: document parsing error: invalid JSON', NULL, NULL),
        ('signature', '', '', 1, 1, 'Invalid signature: bad packet', NULL, NULL),
        ('digest', '', '', 1, 1, 'SHA-256 mismatch: expected a, got b', NULL, NULL),
        ('warning', '', '', 1, 1, NULL, 'SHA-512 mismatch: expected a, got b', NULL),
        ('missing', '', '', 0, 1, NULL, NULL, NULL),
        ('retrieval', '', '', 1, 1, NULL, NULL, 'HTTP 503')").await.unwrap();
    Migrator::up(&db, None).await.unwrap();
    let checks = async |id| super::load_document(&db, id).await.unwrap().unwrap().checks;
    let parsed = checks("parsed").await;
    assert_eq!(parsed.parsing.status, CheckStatus::Passed);
    assert_eq!(parsed.signature.status, CheckStatus::NotEvaluated);
    assert_eq!(parsed.digest.status, CheckStatus::NotEvaluated);
    let parse = checks("parse").await;
    assert_eq!(parse.retrieval.status, CheckStatus::Passed);
    assert_eq!(parse.parsing.status, CheckStatus::Failed);
    assert_eq!(parse.signature.status, CheckStatus::NotEvaluated);
    assert_eq!(parse.digest.status, CheckStatus::NotEvaluated);
    assert_eq!(
        checks("signature").await.signature.status,
        CheckStatus::Failed
    );
    assert_eq!(checks("digest").await.digest.status, CheckStatus::Failed);
    assert_eq!(checks("warning").await.digest.status, CheckStatus::Warning);
    assert_eq!(
        checks("missing").await.signature.status,
        CheckStatus::Missing
    );
    let retrieval = checks("retrieval").await;
    assert_eq!(retrieval.retrieval.status, CheckStatus::Failed);
    assert_eq!(retrieval.parsing.status, CheckStatus::NotEvaluated);
    let summary = super::build_summary_from_db(&db, "example.com")
        .await
        .unwrap()
        .checks
        .unwrap();
    assert_eq!(summary.retrieval.passed, 6);
    assert_eq!(summary.retrieval.failed, 1);
    assert_eq!(summary.parsing.failed, 1);
    assert_eq!(summary.signature.failed, 1);
    assert_eq!(summary.digest.failed, 1);
    assert_eq!(summary.digest.warning, 1);
}

/// Pagination and aggregate metrics classify independent outcomes identically, including overlaps.
#[tokio::test]
async fn check_filters_match_summary_counts() {
    let db = database("sqlite::memory:", None).await;
    let fixtures = [
        ("good", successful_checks()),
        (
            "unsigned",
            DocumentChecks {
                signature: CheckOutcome::new(CheckStatus::Missing),
                ..successful_checks()
            },
        ),
        (
            "no-digests",
            DocumentChecks {
                digest: CheckOutcome::new(CheckStatus::Missing),
                ..successful_checks()
            },
        ),
        (
            "both-fail",
            DocumentChecks {
                signature: CheckOutcome::failed("bad signature"),
                digest: CheckOutcome::failed("bad digest"),
                ..successful_checks()
            },
        ),
        (
            "digest-fail",
            DocumentChecks {
                digest: CheckOutcome::failed("bad digest"),
                ..successful_checks()
            },
        ),
        (
            "digest-warning",
            DocumentChecks {
                digest: CheckOutcome {
                    status: CheckStatus::Warning,
                    message: Some("one digest mismatched".into()),
                    ..Default::default()
                },
                ..successful_checks()
            },
        ),
        ("unknown", DocumentChecks::default()),
    ];
    for (id, checks) in fixtures {
        db.execute_raw(Statement::from_sql_and_values(DbBackend::Sqlite,
            "INSERT INTO documents (tracking_id, title, url, signature_present) VALUES (?, '', ?, 0)", [id.into(), format!("https://example.com/{id}.json").into()],
        )).await.unwrap();
        set_checks(&db, id, &checks).await;
    }
    let summary = super::document_check_summary(&db).await.unwrap();
    for (stage, counts) in [
        ("retrieval", summary.retrieval),
        ("parsing", summary.parsing),
        ("signature", summary.signature),
        ("digest", summary.digest),
    ] {
        assert_eq!(
            counts.passed + counts.failed + counts.warning + counts.missing + counts.not_evaluated,
            7
        );
        for (status, count) in [
            ("passed", counts.passed),
            ("failed", counts.failed),
            ("warning", counts.warning),
            ("missing", counts.missing),
            ("not_evaluated", counts.not_evaluated),
        ] {
            let filter = format!("{stage}-{status}");
            let page = super::load_documents_paginated(&db, 0, 1, Some(&filter))
                .await
                .unwrap();
            assert_eq!(page.total, count, "{filter}");
            assert_eq!(page.items.len() as u64, count.min(1), "{filter}");
        }
    }
    for (filter, count) in [
        ("document-issues", 5),
        ("signature-errors", 1),
        ("digest-errors", 2),
        ("digest-warnings", 1),
        ("missing-signatures", 1),
        ("missing-digests", 1),
        ("not-evaluated", 1),
        ("passing", 3),
        ("failing", 2),
        ("warnings", 3),
    ] {
        assert_eq!(
            super::load_documents_paginated(&db, 0, 10, Some(filter))
                .await
                .unwrap()
                .total,
            count,
            "{filter}"
        );
    }
    let second = super::load_documents_paginated(&db, 1, 1, Some("digest-errors"))
        .await
        .unwrap();
    assert_eq!(second.total, 2);
    assert_eq!(second.items[0].tracking_id, "digest-fail");
}

/// A failed retrieval hides stale checks and profile results; a later validation replaces them.
#[tokio::test]
async fn retrieval_failure_invalidates_checks_until_recovery() {
    let db = database("sqlite::memory:", None).await;
    db.execute_unprepared("INSERT INTO documents (tracking_id, title, url, signature_present, basic_passed, basic_test_count, basic_failing_test_count, basic_error_count)
        VALUES ('example', 'Example', 'https://example.com/example.json', 1, 1, 10, 0, 0)").await.unwrap();
    set_checks(&db, "example", &successful_checks()).await;
    let original = super::load_document(&db, "example").await.unwrap().unwrap();
    assert!(original.profiles.basic.is_some());
    super::save_retrieval_errors(&db, &[(original.url.clone(), "HTTP 503".into())])
        .await
        .unwrap();
    let failed = super::load_document(&db, "example").await.unwrap().unwrap();
    assert_eq!(failed.checks.retrieval.status, CheckStatus::Failed);
    assert_eq!(failed.checks.signature.status, CheckStatus::NotEvaluated);
    assert!(failed.profiles.basic.is_none());
    let summary = super::build_summary_from_db(&db, "example.com")
        .await
        .unwrap();
    assert!(summary.profiles.basic.is_none());
    assert_eq!(summary.checks.unwrap().signature.not_evaluated, 1);
    super::save_documents(&db, &[original]).await.unwrap();
    let recovered = super::load_document(&db, "example").await.unwrap().unwrap();
    assert!(recovered.retrieval_error.is_none());
    assert_eq!(recovered.checks, successful_checks());
    assert!(recovered.profiles.basic.is_some());
    assert_eq!(
        super::build_summary_from_db(&db, "example.com")
            .await
            .unwrap()
            .profiles
            .basic
            .unwrap()
            .pass_rate,
        1.0
    );
}

/// Older outcomes remain readable and newly recorded details survive database loading.
#[tokio::test]
async fn check_details_round_trip() {
    let legacy: CheckOutcome =
        serde_json::from_str(r#"{"status":"passed","message":null}"#).unwrap();
    assert!(legacy.details.is_empty());
    let db = database("sqlite::memory:", None).await;
    db.execute_raw(Statement::from_string(DbBackend::Sqlite,
        "INSERT INTO documents (tracking_id, title, url, signature_present) VALUES ('details', '', 'https://example.com/details.json', 1)"
    )).await.unwrap();
    let mut checks = successful_checks();
    checks.signature.details.push(CheckDetail {
        label: "OpenPGP signature".into(),
        value: "-----BEGIN PGP SIGNATURE-----\nexample\n-----END PGP SIGNATURE-----".into(),
    });
    checks.digest.details.push(CheckDetail {
        label: "SHA-256 · Published".into(),
        value: "0123456789abcdef".into(),
    });
    set_checks(&db, "details", &checks).await;
    let page = super::load_documents_paginated(&db, 0, 10, None)
        .await
        .unwrap();
    assert_eq!(page.items[0].checks, checks);
}

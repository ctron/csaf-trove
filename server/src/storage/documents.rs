use std::collections::HashMap;

use anyhow::Result;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, DbBackend,
    EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Set, Statement,
    TransactionTrait,
};

use csaf_trove_common::Paginated;
use csaf_trove_entity::{check_failure, document, provider_info, revision_history, sync_run};

use crate::models::result::{
    DocumentCheckFailure, DocumentProfileDetail, DocumentProfileResults, DocumentValidation,
    FailingTest, ProfileResults, ProfileSummary, ProviderSummary, RevisionEntry, SignatureSummary,
};

/// Persisted provider metadata fields for aggregator generation.
#[derive(Debug, Clone)]
pub struct ProviderInfo {
    /// Canonical URL of the provider's `provider-metadata.json`.
    pub canonical_url: String,
    /// Publisher name.
    pub publisher_name: String,
    /// Publisher category (e.g. `"vendor"`).
    pub publisher_category: String,
    /// Publisher namespace URI.
    pub publisher_namespace: String,
    /// Role of the issuing party (e.g. `"csaf_provider"`).
    pub role: Option<String>,
    /// Whether the provider consents to being listed by aggregators.
    pub list_on_aggregators: bool,
    /// Whether the provider consents to being mirrored by aggregators.
    pub mirror_on_aggregators: bool,
    /// When the provider metadata was last updated.
    pub last_updated: String,
}

/// Returns the number of documents stored for a provider.
pub async fn document_count(db: &DatabaseConnection) -> Result<u64> {
    let count = document::Entity::find().count(db).await?;
    Ok(count)
}

/// Returns the URL for a document by tracking ID.
pub async fn document_url(db: &DatabaseConnection, tracking_id: &str) -> Result<Option<String>> {
    let result: Option<(String,)> = document::Entity::find()
        .filter(document::Column::TrackingId.eq(tracking_id))
        .select_only()
        .column(document::Column::Url)
        .into_tuple()
        .one(db)
        .await?;
    Ok(result.map(|(url,)| url))
}

/// Upserts document validation results for a provider, preserving documents
/// not in the batch.
pub async fn save_documents(
    db: &DatabaseConnection,
    documents: &[DocumentValidation],
) -> Result<u64> {
    let txn = db.begin().await?;

    for doc in documents {
        let existing_ids: Vec<i64> = document::Entity::find()
            .filter(document::Column::TrackingId.eq(&doc.tracking_id))
            .select_only()
            .column(document::Column::Id)
            .into_tuple()
            .all(&txn)
            .await?;

        if !existing_ids.is_empty() {
            check_failure::Entity::delete_many()
                .filter(check_failure::Column::DocumentId.is_in(existing_ids.clone()))
                .exec(&txn)
                .await?;

            revision_history::Entity::delete_many()
                .filter(revision_history::Column::DocumentId.is_in(existing_ids))
                .exec(&txn)
                .await?;

            document::Entity::delete_many()
                .filter(document::Column::TrackingId.eq(&doc.tracking_id))
                .exec(&txn)
                .await?;
        }

        let (bp, bec, bwc, bic) = profile_to_cols(doc.profiles.basic.as_ref());
        let (ep, eec, ewc, eic) = profile_to_cols(doc.profiles.extended.as_ref());
        let (fp, fec, fwc, fic) = profile_to_cols(doc.profiles.full.as_ref());

        let new_doc = document::ActiveModel {
            tracking_id: Set(doc.tracking_id.clone()),
            title: Set(doc.title.clone()),
            url: Set(doc.url.clone()),
            basic_passed: Set(bp),
            basic_error_count: Set(bec),
            basic_warning_count: Set(bwc),
            basic_info_count: Set(bic),
            extended_passed: Set(ep),
            extended_error_count: Set(eec),
            extended_warning_count: Set(ewc),
            extended_info_count: Set(eic),
            full_passed: Set(fp),
            full_error_count: Set(fec),
            full_warning_count: Set(fwc),
            full_info_count: Set(fic),
            signature_present: Set(doc.signature_present as i32),
            signature_error: Set(doc.signature_error.clone()),
            category: Set(doc.category.clone()),
            publisher_name: Set(doc.publisher_name.clone()),
            initial_release_date: Set(doc.initial_release_date.clone()),
            current_release_date: Set(doc.current_release_date.clone()),
            status: Set(doc.status.clone()),
            revision: Set(doc.revision.clone()),
            aggregate_severity: Set(doc.aggregate_severity.clone()),
            csaf_version: Set(doc.csaf_version.clone()),
            ..Default::default()
        };

        let inserted = new_doc.insert(&txn).await?;
        let doc_id = inserted.id;

        for (profile, detail) in [
            ("basic", &doc.profiles.basic),
            ("extended", &doc.profiles.extended),
            ("full", &doc.profiles.full),
        ] {
            if let Some(d) = detail {
                let failures: Vec<check_failure::ActiveModel> = d
                    .failing_tests
                    .iter()
                    .map(|f| check_failure::ActiveModel {
                        document_id: Set(doc_id),
                        profile: Set(profile.to_string()),
                        test_id: Set(f.test_id.clone()),
                        message: Set(f.message.clone()),
                        severity: Set(f.severity.clone()),
                        ..Default::default()
                    })
                    .collect();

                if !failures.is_empty() {
                    check_failure::Entity::insert_many(failures)
                        .exec(&txn)
                        .await?;
                }
            }
        }

        let revisions: Vec<revision_history::ActiveModel> = doc
            .revision_history
            .iter()
            .map(|r| revision_history::ActiveModel {
                document_id: Set(doc_id),
                version: Set(r.number.clone()),
                date: Set(r.date.clone()),
                summary: Set(r.summary.clone()),
                ..Default::default()
            })
            .collect();

        if !revisions.is_empty() {
            revision_history::Entity::insert_many(revisions)
                .exec(&txn)
                .await?;
        }
    }

    txn.commit().await?;

    let total = document::Entity::find().count(db).await?;
    Ok(total)
}

/// Loads a paginated, optionally filtered list of document validation results.
pub async fn load_documents_paginated(
    db: &DatabaseConnection,
    offset: u64,
    limit: u64,
    status_filter: Option<&str>,
) -> Result<Paginated<DocumentValidation>> {
    let mut query = document::Entity::find();

    match status_filter {
        Some("failing") => {
            query = query.filter(
                Condition::any()
                    .add(document::Column::BasicPassed.eq(0))
                    .add(document::Column::ExtendedPassed.eq(0))
                    .add(document::Column::FullPassed.eq(0))
                    .add(document::Column::SignatureError.is_not_null()),
            );
        }
        Some("passing") => {
            query = query.filter(
                Condition::all()
                    .add(
                        Condition::any()
                            .add(document::Column::BasicPassed.is_null())
                            .add(document::Column::BasicPassed.eq(1)),
                    )
                    .add(
                        Condition::any()
                            .add(document::Column::ExtendedPassed.is_null())
                            .add(document::Column::ExtendedPassed.eq(1)),
                    )
                    .add(
                        Condition::any()
                            .add(document::Column::FullPassed.is_null())
                            .add(document::Column::FullPassed.eq(1)),
                    )
                    .add(document::Column::SignatureError.is_null()),
            );
        }
        _ => {}
    }

    let total = query.clone().count(db).await?;

    let docs = query
        .order_by_asc(document::Column::TrackingId)
        .offset(offset)
        .limit(limit)
        .all(db)
        .await?;

    let items = load_failures_for_docs(db, &docs).await?;

    Ok(Paginated {
        items,
        total,
        offset,
        limit,
    })
}

/// Loads a single document by tracking ID with all its check failures.
pub async fn load_document(
    db: &DatabaseConnection,
    tracking_id: &str,
) -> Result<Option<DocumentValidation>> {
    let doc = document::Entity::find()
        .filter(document::Column::TrackingId.eq(tracking_id))
        .one(db)
        .await?;

    let Some(doc) = doc else {
        return Ok(None);
    };

    let items = load_failures_for_docs(db, &[doc]).await?;
    Ok(items.into_iter().next())
}

/// Loads check failures and revision history for a batch of documents and
/// assembles `DocumentValidation` values.
async fn load_failures_for_docs(
    db: &DatabaseConnection,
    doc_models: &[document::Model],
) -> Result<Vec<DocumentValidation>> {
    if doc_models.is_empty() {
        return Ok(vec![]);
    }

    let ids: Vec<i64> = doc_models.iter().map(|d| d.id).collect();

    let failures = check_failure::Entity::find()
        .filter(check_failure::Column::DocumentId.is_in(ids.clone()))
        .order_by_asc(check_failure::Column::DocumentId)
        .order_by_asc(check_failure::Column::Id)
        .all(db)
        .await?;

    let mut failure_map: HashMap<i64, Vec<(String, String, String, String)>> = HashMap::new();
    for f in failures {
        failure_map
            .entry(f.document_id)
            .or_default()
            .push((f.profile, f.test_id, f.message, f.severity));
    }

    let revisions = revision_history::Entity::find()
        .filter(revision_history::Column::DocumentId.is_in(ids))
        .order_by_asc(revision_history::Column::DocumentId)
        .order_by_asc(revision_history::Column::Id)
        .all(db)
        .await?;

    let mut revision_map: HashMap<i64, Vec<RevisionEntry>> = HashMap::new();
    for r in revisions {
        revision_map
            .entry(r.document_id)
            .or_default()
            .push(RevisionEntry {
                number: r.version,
                date: r.date,
                summary: r.summary,
            });
    }

    let items = doc_models
        .iter()
        .map(|doc| {
            let doc_failures = failure_map.get(&doc.id);
            DocumentValidation {
                tracking_id: doc.tracking_id.clone(),
                title: doc.title.clone(),
                url: doc.url.clone(),
                profiles: DocumentProfileResults {
                    basic: cols_to_profile(
                        doc.basic_passed,
                        doc.basic_error_count,
                        doc.basic_warning_count,
                        doc.basic_info_count,
                        doc_failures,
                        "basic",
                    ),
                    extended: cols_to_profile(
                        doc.extended_passed,
                        doc.extended_error_count,
                        doc.extended_warning_count,
                        doc.extended_info_count,
                        doc_failures,
                        "extended",
                    ),
                    full: cols_to_profile(
                        doc.full_passed,
                        doc.full_error_count,
                        doc.full_warning_count,
                        doc.full_info_count,
                        doc_failures,
                        "full",
                    ),
                },
                signature_error: doc.signature_error.clone(),
                signature_present: doc.signature_present != 0,
                category: doc.category.clone(),
                publisher_name: doc.publisher_name.clone(),
                initial_release_date: doc.initial_release_date.clone(),
                current_release_date: doc.current_release_date.clone(),
                status: doc.status.clone(),
                revision: doc.revision.clone(),
                aggregate_severity: doc.aggregate_severity.clone(),
                csaf_version: doc.csaf_version.clone(),
                revision_history: revision_map.get(&doc.id).cloned().unwrap_or_default(),
                version_count: None,
            }
        })
        .collect();

    Ok(items)
}

/// Converts a `DocumentProfileDetail` into column values for the documents table.
fn profile_to_cols(
    detail: Option<&DocumentProfileDetail>,
) -> (Option<i32>, Option<i64>, Option<i64>, Option<i64>) {
    match detail {
        Some(d) => (
            Some(d.passed as i32),
            Some(d.error_count as i64),
            Some(d.warning_count as i64),
            Some(d.info_count as i64),
        ),
        None => (None, None, None, None),
    }
}

/// Reconstructs a `DocumentProfileDetail` from column values and loaded failures.
fn cols_to_profile(
    passed: Option<i32>,
    error_count: Option<i64>,
    warning_count: Option<i64>,
    info_count: Option<i64>,
    failures: Option<&Vec<(String, String, String, String)>>,
    profile: &str,
) -> Option<DocumentProfileDetail> {
    let passed_val = passed?;
    let failing_tests = failures
        .map(|fs| {
            fs.iter()
                .filter(|(p, _, _, _)| p == profile)
                .map(|(_, test_id, message, severity)| DocumentCheckFailure {
                    test_id: test_id.clone(),
                    message: message.clone(),
                    severity: severity.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    Some(DocumentProfileDetail {
        passed: passed_val != 0,
        error_count: error_count.unwrap_or(0) as u64,
        warning_count: warning_count.unwrap_or(0) as u64,
        info_count: info_count.unwrap_or(0) as u64,
        failing_tests,
    })
}

/// Computes a `ProviderSummary` from all documents in the database.
pub async fn build_summary_from_db(
    db: &DatabaseConnection,
    domain: &str,
) -> Result<ProviderSummary> {
    let result = db
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT
                COUNT(*) AS total,
                SUM(CASE WHEN basic_passed = 1 THEN 1 ELSE 0 END) AS bv,
                SUM(CASE WHEN basic_passed = 0 THEN 1 ELSE 0 END) AS bi,
                SUM(CASE WHEN extended_passed = 1 THEN 1 ELSE 0 END) AS ev,
                SUM(CASE WHEN extended_passed = 0 THEN 1 ELSE 0 END) AS ei,
                SUM(CASE WHEN full_passed = 1 THEN 1 ELSE 0 END) AS fv,
                SUM(CASE WHEN full_passed = 0 THEN 1 ELSE 0 END) AS fi,
                SUM(CASE WHEN signature_present = 1 AND signature_error IS NULL THEN 1 ELSE 0 END) AS sv,
                SUM(CASE WHEN signature_present = 1 AND signature_error IS NOT NULL THEN 1 ELSE 0 END) AS si,
                SUM(CASE WHEN signature_present = 0 THEN 1 ELSE 0 END) AS sm
            FROM documents",
        ))
        .await?;

    let (total, basic, extended, full, signatures) = match result {
        Some(row) => {
            let total: i64 = row.try_get("", "total")?;
            let bv: Option<i64> = row.try_get("", "bv")?;
            let bi: Option<i64> = row.try_get("", "bi")?;
            let ev: Option<i64> = row.try_get("", "ev")?;
            let ei: Option<i64> = row.try_get("", "ei")?;
            let fv: Option<i64> = row.try_get("", "fv")?;
            let fi: Option<i64> = row.try_get("", "fi")?;
            let sv: Option<i64> = row.try_get("", "sv")?;
            let si: Option<i64> = row.try_get("", "si")?;
            let sm: Option<i64> = row.try_get("", "sm")?;
            (
                total as u64,
                build_profile_from_counts(bv.unwrap_or(0) as u64, bi.unwrap_or(0) as u64),
                build_profile_from_counts(ev.unwrap_or(0) as u64, ei.unwrap_or(0) as u64),
                build_profile_from_counts(fv.unwrap_or(0) as u64, fi.unwrap_or(0) as u64),
                SignatureSummary {
                    valid: sv.unwrap_or(0) as u64,
                    invalid: si.unwrap_or(0) as u64,
                    missing: sm.unwrap_or(0) as u64,
                },
            )
        }
        None => (
            0,
            build_profile_from_counts(0, 0),
            build_profile_from_counts(0, 0),
            build_profile_from_counts(0, 0),
            SignatureSummary {
                valid: 0,
                invalid: 0,
                missing: 0,
            },
        ),
    };

    let top_failing_tests: Vec<FailingTest> = {
        let rows = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT test_id, COUNT(*) AS cnt, severity
                 FROM check_failures
                 GROUP BY test_id, severity
                 ORDER BY cnt DESC
                 LIMIT 10",
            ))
            .await?;

        rows.iter()
            .map(|row| {
                Ok(FailingTest {
                    test_id: row.try_get("", "test_id")?,
                    count: {
                        let c: i64 = row.try_get("", "cnt")?;
                        c as u64
                    },
                    severity: row.try_get("", "severity")?,
                })
            })
            .collect::<Result<Vec<_>, sea_orm::DbErr>>()?
    };

    let publisher_name: Option<String> = {
        let row = db
            .query_one_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT publisher_name FROM documents
                 WHERE publisher_name IS NOT NULL
                 GROUP BY publisher_name
                 ORDER BY COUNT(*) DESC
                 LIMIT 1",
            ))
            .await?;

        match row {
            Some(r) => r.try_get("", "publisher_name")?,
            None => None,
        }
    };

    Ok(ProviderSummary {
        provider: domain.to_string(),
        publisher_name,
        validated_at: chrono::Utc::now(),
        document_count: total,
        profiles: ProfileResults {
            basic: Some(basic),
            extended: Some(extended),
            full: Some(full),
        },
        signatures: Some(signatures),
        top_failing_tests,
    })
}

/// Builds a `ProfileSummary` from valid and invalid counts.
fn build_profile_from_counts(valid: u64, invalid: u64) -> ProfileSummary {
    let total = valid + invalid;
    let pass_rate = if total > 0 {
        valid as f64 / total as f64
    } else {
        0.0
    };
    ProfileSummary {
        valid,
        invalid,
        pass_rate,
    }
}

/// Records a completed sync run with the number of documents that changed.
pub async fn save_sync_run(
    db: &DatabaseConnection,
    timestamp: &chrono::DateTime<chrono::Utc>,
    documents_changed: u64,
) -> Result<()> {
    sync_run::ActiveModel {
        timestamp: Set(timestamp.to_rfc3339()),
        documents_changed: Set(documents_changed as i64),
        ..Default::default()
    }
    .insert(db)
    .await?;
    Ok(())
}

/// Loads the most recent sync runs for a provider.
pub async fn load_sync_runs(
    db: &DatabaseConnection,
    max_entries: u64,
) -> Result<Vec<csaf_trove_common::CommitInfo>> {
    let rows = sync_run::Entity::find()
        .order_by_desc(sync_run::Column::Id)
        .limit(max_entries)
        .all(db)
        .await?;

    let entries = rows
        .into_iter()
        .map(|row| {
            let timestamp = chrono::DateTime::parse_from_rfc3339(&row.timestamp)
                .map(|dt| {
                    time::OffsetDateTime::from_unix_timestamp(dt.timestamp())
                        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
                })
                .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
            csaf_trove_common::CommitInfo {
                id: row.id.to_string(),
                message: String::new(),
                timestamp,
                files_changed: row.documents_changed as usize,
            }
        })
        .collect();

    Ok(entries)
}

/// Loads a paginated list of sync runs for a provider.
pub async fn load_sync_runs_paginated(
    db: &DatabaseConnection,
    offset: u64,
    limit: u64,
) -> Result<Paginated<csaf_trove_common::CommitInfo>> {
    let total = sync_run::Entity::find().count(db).await?;

    let rows = sync_run::Entity::find()
        .order_by_desc(sync_run::Column::Id)
        .offset(offset)
        .limit(limit)
        .all(db)
        .await?;

    let items = rows
        .into_iter()
        .map(|row| {
            let timestamp = chrono::DateTime::parse_from_rfc3339(&row.timestamp)
                .map(|dt| {
                    time::OffsetDateTime::from_unix_timestamp(dt.timestamp())
                        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
                })
                .unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
            csaf_trove_common::CommitInfo {
                id: row.id.to_string(),
                message: String::new(),
                timestamp,
                files_changed: row.documents_changed as usize,
            }
        })
        .collect();

    Ok(Paginated {
        items,
        total,
        offset,
        limit,
    })
}

/// Upserts provider metadata info for aggregator generation.
pub async fn save_provider_info(db: &DatabaseConnection, info: &ProviderInfo) -> Result<()> {
    let model = provider_info::ActiveModel {
        id: Set(1),
        canonical_url: Set(info.canonical_url.clone()),
        publisher_name: Set(info.publisher_name.clone()),
        publisher_category: Set(info.publisher_category.clone()),
        publisher_namespace: Set(info.publisher_namespace.clone()),
        role: Set(info.role.clone()),
        list_on_aggregators: Set(info.list_on_aggregators as i32),
        mirror_on_aggregators: Set(info.mirror_on_aggregators as i32),
        last_updated: Set(info.last_updated.clone()),
    };

    provider_info::Entity::insert(model)
        .on_conflict(
            sea_orm::sea_query::OnConflict::column(provider_info::Column::Id)
                .update_columns([
                    provider_info::Column::CanonicalUrl,
                    provider_info::Column::PublisherName,
                    provider_info::Column::PublisherCategory,
                    provider_info::Column::PublisherNamespace,
                    provider_info::Column::Role,
                    provider_info::Column::ListOnAggregators,
                    provider_info::Column::MirrorOnAggregators,
                    provider_info::Column::LastUpdated,
                ])
                .to_owned(),
        )
        .exec(db)
        .await?;

    Ok(())
}

/// Loads the persisted provider metadata info, if available.
pub async fn load_provider_info(db: &DatabaseConnection) -> Result<Option<ProviderInfo>> {
    let row = provider_info::Entity::find_by_id(1).one(db).await?;

    Ok(row.map(|r| ProviderInfo {
        canonical_url: r.canonical_url,
        publisher_name: r.publisher_name,
        publisher_category: r.publisher_category,
        publisher_namespace: r.publisher_namespace,
        role: r.role,
        list_on_aggregators: r.list_on_aggregators != 0,
        mirror_on_aggregators: r.mirror_on_aggregators != 0,
        last_updated: r.last_updated,
    }))
}

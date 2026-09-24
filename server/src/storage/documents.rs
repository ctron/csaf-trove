mod test;

use super::git_repo::document_version_counts;
use anyhow::{Error, Result};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, DbBackend,
    EntityTrait, FromQueryResult, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect, Set,
    Statement, TransactionTrait, sea_query::Expr,
};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use crate::models::result::{
    DocumentCheckFailure, DocumentProfileDetail, DocumentProfileResults, DocumentValidation,
    FailingTest, ProfileResults, ProfileSummary, ProviderSummary, RevisionEntry, SignatureSummary,
};
use csaf_trove_common::{CommitInfo, Paginated, SyncPoint};
use csaf_trove_entity::{check_failure, document, provider_info, revision_history, sync_run};
use time::OffsetDateTime;
use tokio::task::spawn_blocking;

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

/// Applies the lossy filename transformation from CSAF spec section 5.1:
/// lowercase, then replace any character not in `[a-z0-9+-]` with `_`.
fn lossy_tracking_id(tracking_id: &str) -> String {
    tracking_id
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '+' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Extracts a synthetic tracking ID from a CSAF document URL by stripping
/// the path and `.json` suffix.
fn tracking_id_from_url(url: &str) -> String {
    url.rsplit('/')
        .next()
        .unwrap_or(url)
        .trim_end_matches(".json")
        .to_string()
}

/// Deletes all document rows (and their check failures and revision history) for a provider.
pub async fn delete_all_documents(db: &DatabaseConnection) -> Result<()> {
    check_failure::Entity::delete_many().exec(db).await?;
    revision_history::Entity::delete_many().exec(db).await?;
    document::Entity::delete_many().exec(db).await?;
    Ok(())
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
        let lossy_id = lossy_tracking_id(&doc.tracking_id);
        let condition = if lossy_id != doc.tracking_id {
            Condition::any()
                .add(document::Column::TrackingId.eq(&doc.tracking_id))
                .add(document::Column::TrackingId.eq(&lossy_id))
        } else {
            Condition::any().add(document::Column::TrackingId.eq(&doc.tracking_id))
        };

        let existing_ids: Vec<i64> = document::Entity::find()
            .filter(condition.clone())
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
                .filter(condition)
                .exec(&txn)
                .await?;
        }

        let basic = profile_to_cols(doc.profiles.basic.as_ref());
        let extended = profile_to_cols(doc.profiles.extended.as_ref());
        let full = profile_to_cols(doc.profiles.full.as_ref());

        let new_doc = document::ActiveModel {
            tracking_id: Set(doc.tracking_id.clone()),
            title: Set(doc.title.clone()),
            url: Set(doc.url.clone()),
            basic_passed: Set(basic.passed),
            basic_error_count: Set(basic.error_count),
            basic_warning_count: Set(basic.warning_count),
            basic_info_count: Set(basic.info_count),
            basic_test_count: Set(basic.test_count),
            basic_failing_test_count: Set(basic.failing_test_count),
            extended_passed: Set(extended.passed),
            extended_error_count: Set(extended.error_count),
            extended_warning_count: Set(extended.warning_count),
            extended_info_count: Set(extended.info_count),
            extended_test_count: Set(extended.test_count),
            extended_failing_test_count: Set(extended.failing_test_count),
            full_passed: Set(full.passed),
            full_error_count: Set(full.error_count),
            full_warning_count: Set(full.warning_count),
            full_info_count: Set(full.info_count),
            full_test_count: Set(full.test_count),
            full_failing_test_count: Set(full.failing_test_count),
            signature_present: Set(doc.signature_present as i32),
            signature_error: Set(doc.signature_error.clone()),
            signature_warning: Set(doc.signature_warning.clone()),
            category: Set(doc.category.clone()),
            publisher_name: Set(doc.publisher_name.clone()),
            initial_release_date: Set(doc.initial_release_date.clone()),
            current_release_date: Set(doc.current_release_date.clone()),
            status: Set(doc.status.clone()),
            revision: Set(doc.revision.clone()),
            aggregate_severity: Set(doc.aggregate_severity.clone()),
            csaf_version: Set(doc.csaf_version.clone()),
            retrieval_error: Set(doc.retrieval_error.clone()),
            version_count: Set(doc.version_count as i32),
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
                    .add(document::Column::SignatureError.is_not_null())
                    .add(document::Column::RetrievalError.is_not_null()),
            );
        }
        Some("errors") => {
            query = query.filter(document::Column::RetrievalError.is_not_null());
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
                    basic: cols_to_profile(doc, doc_failures, "basic"),
                    extended: cols_to_profile(doc, doc_failures, "extended"),
                    full: cols_to_profile(doc, doc_failures, "full"),
                },
                signature_error: doc.signature_error.clone(),
                signature_warning: doc.signature_warning.clone(),
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
                version_count: doc.version_count as u32,
                retrieval_error: doc.retrieval_error.clone(),
            }
        })
        .collect();

    Ok(items)
}

#[derive(Default)]
struct ProfileColumns {
    passed: Option<i32>,
    error_count: Option<i64>,
    warning_count: Option<i64>,
    info_count: Option<i64>,
    test_count: Option<i64>,
    failing_test_count: Option<i64>,
}

fn profile_to_cols(detail: Option<&DocumentProfileDetail>) -> ProfileColumns {
    match detail {
        Some(d) => ProfileColumns {
            passed: Some(d.passed as i32),
            error_count: Some(d.error_count as i64),
            warning_count: Some(d.warning_count as i64),
            info_count: Some(d.info_count as i64),
            test_count: Some(d.total_tests as i64),
            failing_test_count: Some(d.failing_test_count as i64),
        },
        None => ProfileColumns::default(),
    }
}

fn cols_to_profile(
    doc: &document::Model,
    failures: Option<&Vec<(String, String, String, String)>>,
    profile: &str,
) -> Option<DocumentProfileDetail> {
    let cols = match profile {
        "basic" => ProfileColumns {
            passed: doc.basic_passed,
            error_count: doc.basic_error_count,
            warning_count: doc.basic_warning_count,
            info_count: doc.basic_info_count,
            test_count: doc.basic_test_count,
            failing_test_count: doc.basic_failing_test_count,
        },
        "extended" => ProfileColumns {
            passed: doc.extended_passed,
            error_count: doc.extended_error_count,
            warning_count: doc.extended_warning_count,
            info_count: doc.extended_info_count,
            test_count: doc.extended_test_count,
            failing_test_count: doc.extended_failing_test_count,
        },
        "full" => ProfileColumns {
            passed: doc.full_passed,
            error_count: doc.full_error_count,
            warning_count: doc.full_warning_count,
            info_count: doc.full_info_count,
            test_count: doc.full_test_count,
            failing_test_count: doc.full_failing_test_count,
        },
        _ => return None,
    };
    let passed_val = cols.passed?;
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
        error_count: cols.error_count.unwrap_or(0) as u64,
        warning_count: cols.warning_count.unwrap_or(0) as u64,
        info_count: cols.info_count.unwrap_or(0) as u64,
        total_tests: cols.test_count.unwrap_or(0) as u64,
        failing_test_count: cols.failing_test_count.unwrap_or(0) as u64,
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
                SUM(COALESCE(basic_test_count, 0) - COALESCE(basic_failing_test_count, 0)) AS bv,
                SUM(COALESCE(basic_failing_test_count, 0)) AS bi,
                SUM(COALESCE(extended_test_count, 0) - COALESCE(extended_failing_test_count, 0)) AS ev,
                SUM(COALESCE(extended_failing_test_count, 0)) AS ei,
                SUM(COALESCE(full_test_count, 0) - COALESCE(full_failing_test_count, 0)) AS fv,
                SUM(COALESCE(full_failing_test_count, 0)) AS fi,
                SUM(CASE WHEN signature_present = 1 AND signature_error IS NULL THEN 1 ELSE 0 END) AS sv,
                SUM(CASE WHEN signature_present = 1 AND signature_error IS NOT NULL THEN 1 ELSE 0 END) AS si,
                SUM(CASE WHEN signature_present = 0 THEN 1 ELSE 0 END) AS sm,
                SUM(CASE WHEN retrieval_error IS NOT NULL THEN 1 ELSE 0 END) AS re
            FROM documents",
        ))
        .await?;

    let (total, basic, extended, full, signatures, retrieval_errors) = match result {
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
            let re: Option<i64> = row.try_get("", "re")?;
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
                re.unwrap_or(0) as u64,
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
            0,
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
        validated_at: OffsetDateTime::now_utc(),
        document_count: total,
        profiles: ProfileResults {
            basic: Some(basic),
            extended: Some(extended),
            full: Some(full),
        },
        signatures: Some(signatures),
        top_failing_tests,
        retrieval_errors,
        note: None,
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

/// Raw row shape for the distribution health aggregation query.
#[derive(Debug, sea_orm::FromQueryResult)]
struct DistributionHealthRow {
    total: i64,
    bv: Option<i64>,
    bi: Option<i64>,
    ev: Option<i64>,
    ei: Option<i64>,
    fv: Option<i64>,
    fi: Option<i64>,
    re: Option<i64>,
}

impl DistributionHealthRow {
    fn pass_rate(valid: Option<i64>, invalid: Option<i64>) -> Option<f64> {
        let v = valid.unwrap_or(0) as u64;
        let i = invalid.unwrap_or(0) as u64;
        let total = v + i;
        if total > 0 {
            Some(v as f64 / total as f64)
        } else {
            None
        }
    }

    fn into_tuple(self) -> (u64, u64, Option<f64>, Option<f64>, Option<f64>) {
        (
            self.total as u64,
            self.re.unwrap_or(0) as u64,
            Self::pass_rate(self.bv, self.bi),
            Self::pass_rate(self.ev, self.ei),
            Self::pass_rate(self.fv, self.fi),
        )
    }
}

/// Computes health metrics for documents whose URL starts with the given prefix.
///
/// Returns `(document_count, retrieval_errors, basic_pass_rate, extended_pass_rate, full_pass_rate)`.
pub async fn distribution_health(
    db: &DatabaseConnection,
    url_prefix: &str,
) -> Result<(u64, u64, Option<f64>, Option<f64>, Option<f64>)> {
    let like_pattern = format!("{url_prefix}%");
    let row = DistributionHealthRow::find_by_statement(Statement::from_sql_and_values(
        DbBackend::Sqlite,
        "SELECT
            COUNT(*) AS total,
            SUM(COALESCE(basic_test_count, 0) - COALESCE(basic_failing_test_count, 0)) AS bv,
            SUM(COALESCE(basic_failing_test_count, 0)) AS bi,
            SUM(COALESCE(extended_test_count, 0) - COALESCE(extended_failing_test_count, 0)) AS ev,
            SUM(COALESCE(extended_failing_test_count, 0)) AS ei,
            SUM(COALESCE(full_test_count, 0) - COALESCE(full_failing_test_count, 0)) AS fv,
            SUM(COALESCE(full_failing_test_count, 0)) AS fi,
            SUM(CASE WHEN retrieval_error IS NOT NULL THEN 1 ELSE 0 END) AS re
        FROM documents
        WHERE url LIKE ?1",
        [like_pattern.into()],
    ))
    .one(db)
    .await?;

    Ok(row
        .map(|r| r.into_tuple())
        .unwrap_or((0, 0, None, None, None)))
}

/// Records a completed sync run with the number of documents that changed.
pub async fn save_sync_run(
    db: &DatabaseConnection,
    timestamp: OffsetDateTime,
    documents_changed: u64,
) -> Result<()> {
    sync_run::ActiveModel {
        timestamp: Set(timestamp),
        documents_changed: Set(documents_changed as i64),
        ..Default::default()
    }
    .insert(db)
    .await?;
    Ok(())
}

/// Loads the most recent sync runs for a provider.
pub async fn load_sync_runs(db: &DatabaseConnection, max_entries: u64) -> Result<Vec<CommitInfo>> {
    let rows = sync_run::Entity::find()
        .order_by_desc(sync_run::Column::Id)
        .limit(max_entries)
        .all(db)
        .await?;

    let entries = rows
        .into_iter()
        .map(|row| CommitInfo {
            id: row.id.to_string(),
            message: String::new(),
            timestamp: row.timestamp,
            files_changed: row.documents_changed as usize,
        })
        .collect();

    Ok(entries)
}

/// Loads a paginated list of sync runs for a provider.
pub async fn load_sync_runs_paginated(
    db: &DatabaseConnection,
    offset: u64,
    limit: u64,
) -> Result<Paginated<CommitInfo>> {
    let total = sync_run::Entity::find().count(db).await?;

    let rows = sync_run::Entity::find()
        .order_by_desc(sync_run::Column::Id)
        .offset(offset)
        .limit(limit)
        .all(db)
        .await?;

    let items = rows
        .into_iter()
        .map(|row| CommitInfo {
            id: row.id.to_string(),
            message: String::new(),
            timestamp: row.timestamp,
            files_changed: row.documents_changed as usize,
        })
        .collect();

    Ok(Paginated {
        items,
        total,
        offset,
        limit,
    })
}

/// Loads the most recent sync runs as sparkline data points in chronological order.
pub async fn load_recent_sync_points(
    db: &DatabaseConnection,
    limit: u64,
) -> Result<Vec<SyncPoint>> {
    let rows = sync_run::Entity::find()
        .order_by_desc(sync_run::Column::Id)
        .limit(limit)
        .all(db)
        .await?;

    let mut points: Vec<SyncPoint> = rows
        .into_iter()
        .map(|row| SyncPoint {
            timestamp: row.timestamp,
            count: row.documents_changed as u64,
        })
        .collect();
    points.reverse();

    Ok(points)
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

/// Persists retrieval errors as document entries.
///
/// For each `(url, error)` pair, either updates the `retrieval_error` column on an
/// existing document (matched by tracking ID derived from the URL) or inserts a stub
/// row when no prior document exists.
pub async fn save_retrieval_errors(
    db: &DatabaseConnection,
    errors: &[(String, String)],
) -> Result<()> {
    let txn = db.begin().await?;

    for (url, error) in errors {
        let tracking_id = tracking_id_from_url(url);

        let existing = document::Entity::find()
            .filter(document::Column::TrackingId.eq(&tracking_id))
            .one(&txn)
            .await?;

        if let Some(doc) = existing {
            let mut active: document::ActiveModel = doc.into();
            active.retrieval_error = Set(Some(error.clone()));
            active.update(&txn).await?;
        } else {
            document::ActiveModel {
                tracking_id: Set(tracking_id),
                title: Set("Retrieval failed".to_string()),
                url: Set(url.clone()),
                retrieval_error: Set(Some(error.clone())),
                signature_present: Set(0),
                ..Default::default()
            }
            .insert(&txn)
            .await?;
        }
    }

    txn.commit().await?;
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

/// Backfills `test_count` and `failing_test_count` columns for rows that
/// predate the migration (i.e. where these columns are still NULL).
pub async fn backfill_test_counts(db: &DatabaseConnection) -> Result<()> {
    use csaf::validation::Validatable;
    type Csaf20 = csaf::schema::csaf2_0::schema::CommonSecurityAdvisoryFramework;
    type Csaf21 = csaf::schema::csaf2_1::schema::CommonSecurityAdvisoryFramework;

    let row = db
        .query_one_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT COUNT(*) AS cnt FROM documents WHERE basic_test_count IS NULL AND basic_passed IS NOT NULL",
        ))
        .await?;
    let needs: i64 = row.map(|r| r.try_get("", "cnt").unwrap_or(0)).unwrap_or(0);
    if needs == 0 {
        return Ok(());
    }

    tracing::info!("Backfilling test counts for {needs} documents");

    db.execute_unprepared(
        "UPDATE documents SET
            basic_failing_test_count = COALESCE((SELECT COUNT(DISTINCT test_id) FROM check_failures WHERE document_id = documents.id AND profile = 'basic'), 0),
            extended_failing_test_count = COALESCE((SELECT COUNT(DISTINCT test_id) FROM check_failures WHERE document_id = documents.id AND profile = 'extended'), 0),
            full_failing_test_count = COALESCE((SELECT COUNT(DISTINCT test_id) FROM check_failures WHERE document_id = documents.id AND profile = 'full'), 0)
         WHERE basic_test_count IS NULL AND basic_passed IS NOT NULL",
    )
    .await?;

    for (version_str, basic, extended, full) in [
        (
            "2.0",
            Csaf20::tests_in_preset("basic").map_or(0, |v| v.len()),
            Csaf20::tests_in_preset("extended").map_or(0, |v| v.len()),
            Csaf20::tests_in_preset("full").map_or(0, |v| v.len()),
        ),
        (
            "2.1",
            Csaf21::tests_in_preset("basic").map_or(0, |v| v.len()),
            Csaf21::tests_in_preset("extended").map_or(0, |v| v.len()),
            Csaf21::tests_in_preset("full").map_or(0, |v| v.len()),
        ),
    ] {
        db.execute_unprepared(&format!(
            "UPDATE documents SET basic_test_count = {basic}, extended_test_count = {extended}, full_test_count = {full}
             WHERE csaf_version = '{version_str}' AND basic_test_count IS NULL AND basic_passed IS NOT NULL",
        ))
        .await?;
    }

    tracing::info!("Test count backfill complete");
    Ok(())
}

/// Identifies a document and its current count without loading validation details.
#[derive(FromQueryResult)]
struct DocumentVersionRow {
    /// Indexed primary key used to update this row.
    id: i64,
    /// Advisory URL used to locate the document in Git history.
    url: String,
    /// Previously stored count, used to avoid unnecessary writes.
    version_count: i32,
}

/// Computes version counts from Git history and updates changed rows by primary key.
pub async fn update_version_counts(db: &DatabaseConnection, repo_path: &Path) -> Result<()> {
    if !repo_path.exists() {
        return Ok(());
    }

    let documents = document::Entity::find()
        .select_only()
        .column(document::Column::Id)
        .column(document::Column::Url)
        .column(document::Column::VersionCount)
        .into_model::<DocumentVersionRow>()
        .all(db)
        .await?;

    if documents.is_empty() {
        return Ok(());
    }

    let repo = repo_path.to_path_buf();
    let (documents, counts) = spawn_blocking(move || {
        // Multiple rows may share a URL; count its history only once.
        let urls: HashSet<&str> = documents.iter().map(|doc| doc.url.as_str()).collect();
        let url_refs: Vec<&str> = urls.into_iter().collect();
        let counts = document_version_counts(&repo, &url_refs)?;
        Ok::<_, Error>((documents, counts))
    })
    .await??;

    let txn = db.begin().await?;
    for doc in documents {
        let Some(&count) = counts.get(&doc.url) else {
            continue;
        };
        let count = i32::try_from(count)?;
        if count == doc.version_count {
            continue;
        }
        document::Entity::update_many()
            .col_expr(document::Column::VersionCount, Expr::value(count))
            .filter(document::Column::Id.eq(doc.id))
            .exec(&txn)
            .await?;
    }
    txn.commit().await?;

    Ok(())
}

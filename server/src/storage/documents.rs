use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

use crate::models::{
    result::{
        DocumentCheckFailure, DocumentProfileDetail, DocumentProfileResults, DocumentValidation,
        PaginatedDocuments,
    },
    source::sanitize_domain,
};

/// Opens (or creates) the SQLite database for a provider's document results.
fn open_db(results_dir: &Path, domain: &str) -> Result<Connection> {
    let dir = results_dir.join(sanitize_domain(domain));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("documents.db");
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    create_tables(&conn)?;
    Ok(conn)
}

/// Creates the schema if it does not already exist.
fn create_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS documents (
            id INTEGER PRIMARY KEY,
            tracking_id TEXT NOT NULL,
            title TEXT NOT NULL,
            url TEXT NOT NULL,
            basic_passed INTEGER,
            basic_error_count INTEGER,
            extended_passed INTEGER,
            extended_error_count INTEGER,
            full_passed INTEGER,
            full_error_count INTEGER,
            signature_present INTEGER NOT NULL,
            signature_error TEXT
        );
        CREATE TABLE IF NOT EXISTS check_failures (
            id INTEGER PRIMARY KEY,
            document_id INTEGER NOT NULL REFERENCES documents(id),
            profile TEXT NOT NULL,
            test_id TEXT NOT NULL,
            message TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_documents_tracking_id ON documents(tracking_id);
        CREATE INDEX IF NOT EXISTS idx_check_failures_document_id ON check_failures(document_id);",
    )?;
    Ok(())
}

/// Replaces all stored document results for a provider.
pub fn save_documents(
    results_dir: &Path,
    domain: &str,
    documents: &[DocumentValidation],
) -> Result<()> {
    let conn = open_db(results_dir, domain)?;

    conn.execute_batch("DELETE FROM check_failures; DELETE FROM documents;")?;

    let mut doc_stmt = conn.prepare(
        "INSERT INTO documents (
            tracking_id, title, url,
            basic_passed, basic_error_count,
            extended_passed, extended_error_count,
            full_passed, full_error_count,
            signature_present, signature_error
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )?;

    let mut fail_stmt = conn.prepare(
        "INSERT INTO check_failures (document_id, profile, test_id, message)
         VALUES (?1, ?2, ?3, ?4)",
    )?;

    let tx = conn.unchecked_transaction()?;

    for doc in documents {
        let (bp, bec) = profile_to_cols(doc.profiles.basic.as_ref());
        let (ep, eec) = profile_to_cols(doc.profiles.extended.as_ref());
        let (fp, fec) = profile_to_cols(doc.profiles.full.as_ref());

        doc_stmt.execute(rusqlite::params![
            doc.tracking_id,
            doc.title,
            doc.url,
            bp,
            bec,
            ep,
            eec,
            fp,
            fec,
            doc.signature_present as i32,
            doc.signature_error,
        ])?;

        let doc_id = tx.last_insert_rowid();

        for (profile, detail) in [
            ("basic", &doc.profiles.basic),
            ("extended", &doc.profiles.extended),
            ("full", &doc.profiles.full),
        ] {
            if let Some(d) = detail {
                for f in &d.failing_tests {
                    fail_stmt.execute(rusqlite::params![doc_id, profile, f.test_id, f.message])?;
                }
            }
        }
    }

    tx.commit()?;
    Ok(())
}

/// Loads a paginated, optionally filtered list of document validation results.
pub fn load_documents_paginated(
    results_dir: &Path,
    domain: &str,
    offset: u64,
    limit: u64,
    status_filter: Option<&str>,
) -> Result<Option<PaginatedDocuments>> {
    let db_path = results_dir
        .join(sanitize_domain(domain))
        .join("documents.db");
    if !db_path.exists() {
        return Ok(None);
    }
    let conn = Connection::open(db_path)?;

    let where_clause = status_where_clause(status_filter);

    let total: u64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM documents {where_clause}"),
        [],
        |row| row.get(0),
    )?;

    let mut stmt = conn.prepare(&format!(
        "SELECT id, tracking_id, title, url,
                basic_passed, basic_error_count,
                extended_passed, extended_error_count,
                full_passed, full_error_count,
                signature_present, signature_error
         FROM documents {where_clause}
         ORDER BY tracking_id
         LIMIT ?1 OFFSET ?2"
    ))?;

    let rows = stmt.query_map(rusqlite::params![limit, offset], |row| {
        Ok(DocumentRow {
            id: row.get(0)?,
            tracking_id: row.get(1)?,
            title: row.get(2)?,
            url: row.get(3)?,
            basic_passed: row.get(4)?,
            basic_error_count: row.get(5)?,
            extended_passed: row.get(6)?,
            extended_error_count: row.get(7)?,
            full_passed: row.get(8)?,
            full_error_count: row.get(9)?,
            signature_present: row.get::<_, i32>(10)? != 0,
            signature_error: row.get(11)?,
        })
    })?;

    let doc_rows: Vec<DocumentRow> = rows.collect::<Result<_, _>>()?;
    let items = load_failures_for_docs(&conn, &doc_rows)?;

    Ok(Some(PaginatedDocuments {
        items,
        total,
        offset,
        limit,
    }))
}

/// Loads a single document by tracking ID with all its check failures.
pub fn load_document(
    results_dir: &Path,
    domain: &str,
    tracking_id: &str,
) -> Result<Option<DocumentValidation>> {
    let db_path = results_dir
        .join(sanitize_domain(domain))
        .join("documents.db");
    if !db_path.exists() {
        return Ok(None);
    }
    let conn = Connection::open(db_path)?;

    let row = conn.query_row(
        "SELECT id, tracking_id, title, url,
                basic_passed, basic_error_count,
                extended_passed, extended_error_count,
                full_passed, full_error_count,
                signature_present, signature_error
         FROM documents WHERE tracking_id = ?1",
        [tracking_id],
        |row| {
            Ok(DocumentRow {
                id: row.get(0)?,
                tracking_id: row.get(1)?,
                title: row.get(2)?,
                url: row.get(3)?,
                basic_passed: row.get(4)?,
                basic_error_count: row.get(5)?,
                extended_passed: row.get(6)?,
                extended_error_count: row.get(7)?,
                full_passed: row.get(8)?,
                full_error_count: row.get(9)?,
                signature_present: row.get::<_, i32>(10)? != 0,
                signature_error: row.get(11)?,
            })
        },
    );

    match row {
        Ok(doc_row) => {
            let items = load_failures_for_docs(&conn, &[doc_row])?;
            Ok(items.into_iter().next())
        }
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Intermediate row from the documents table.
struct DocumentRow {
    id: i64,
    tracking_id: String,
    title: String,
    url: String,
    basic_passed: Option<i32>,
    basic_error_count: Option<i64>,
    extended_passed: Option<i32>,
    extended_error_count: Option<i64>,
    full_passed: Option<i32>,
    full_error_count: Option<i64>,
    signature_present: bool,
    signature_error: Option<String>,
}

/// Loads check failures for a batch of document rows and assembles `DocumentValidation` values.
fn load_failures_for_docs(
    conn: &Connection,
    doc_rows: &[DocumentRow],
) -> Result<Vec<DocumentValidation>> {
    if doc_rows.is_empty() {
        return Ok(vec![]);
    }

    let ids: Vec<i64> = doc_rows.iter().map(|r| r.id).collect();
    let placeholders: String = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");

    let mut stmt = conn.prepare(&format!(
        "SELECT document_id, profile, test_id, message
         FROM check_failures
         WHERE document_id IN ({placeholders})
         ORDER BY document_id, id"
    ))?;

    let rows = stmt.query_map(rusqlite::params_from_iter(ids.iter()), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })?;

    let mut failures: std::collections::HashMap<i64, Vec<(String, String, String)>> =
        std::collections::HashMap::new();
    for row in rows {
        let (doc_id, profile, test_id, message) = row?;
        failures
            .entry(doc_id)
            .or_default()
            .push((profile, test_id, message));
    }

    let items = doc_rows
        .iter()
        .map(|doc| {
            let doc_failures = failures.get(&doc.id);
            DocumentValidation {
                tracking_id: doc.tracking_id.clone(),
                title: doc.title.clone(),
                url: doc.url.clone(),
                profiles: DocumentProfileResults {
                    basic: cols_to_profile(
                        doc.basic_passed,
                        doc.basic_error_count,
                        doc_failures,
                        "basic",
                    ),
                    extended: cols_to_profile(
                        doc.extended_passed,
                        doc.extended_error_count,
                        doc_failures,
                        "extended",
                    ),
                    full: cols_to_profile(
                        doc.full_passed,
                        doc.full_error_count,
                        doc_failures,
                        "full",
                    ),
                },
                signature_error: doc.signature_error.clone(),
                signature_present: doc.signature_present,
            }
        })
        .collect();

    Ok(items)
}

/// Converts a `DocumentProfileDetail` into column values for the documents table.
fn profile_to_cols(detail: Option<&DocumentProfileDetail>) -> (Option<i32>, Option<i64>) {
    match detail {
        Some(d) => (Some(d.passed as i32), Some(d.error_count as i64)),
        None => (None, None),
    }
}

/// Reconstructs a `DocumentProfileDetail` from column values and loaded failures.
fn cols_to_profile(
    passed: Option<i32>,
    error_count: Option<i64>,
    failures: Option<&Vec<(String, String, String)>>,
    profile: &str,
) -> Option<DocumentProfileDetail> {
    let passed_val = passed?;
    let failing_tests = failures
        .map(|fs| {
            fs.iter()
                .filter(|(p, _, _)| p == profile)
                .map(|(_, test_id, message)| DocumentCheckFailure {
                    test_id: test_id.clone(),
                    message: message.clone(),
                })
                .collect()
        })
        .unwrap_or_default();

    Some(DocumentProfileDetail {
        passed: passed_val != 0,
        error_count: error_count.unwrap_or(0) as u64,
        failing_tests,
    })
}

/// Builds the SQL WHERE clause for the status filter.
fn status_where_clause(filter: Option<&str>) -> String {
    match filter {
        Some("failing") => "WHERE basic_passed = 0 OR extended_passed = 0 OR full_passed = 0 \
             OR signature_error IS NOT NULL"
            .to_string(),
        Some("passing") => "WHERE (basic_passed IS NULL OR basic_passed = 1) \
             AND (extended_passed IS NULL OR extended_passed = 1) \
             AND (full_passed IS NULL OR full_passed = 1) \
             AND signature_error IS NULL"
            .to_string(),
        _ => String::new(),
    }
}

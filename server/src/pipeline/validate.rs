use std::{collections::HashMap, path::Path, sync::Arc};

use anyhow::Result;
use csaf_walker::{
    check::CheckError,
    common::{
        utils::{openpgp::PublicKey, url::Urlify},
        validate::{
            ValidationOptions, digest::validate_digest, openpgp::validate_signature,
            source::KeySource,
        },
    },
    retrieve::{RetrievedAdvisory, RetrievingVisitor},
    source::{FileSource, Source as CsafSource},
    verification::{
        Csaf, VerificationError, VerifiedAdvisory, VerifyingVisitor,
        check::{Check, CsafValidation},
    },
    walker::Walker,
};
use parking_lot::Mutex;

use crate::{
    AppState,
    models::{
        result::{
            DocumentCheckFailure, DocumentProfileDetail, DocumentProfileResults,
            DocumentValidation, RevisionEntry,
        },
        source::Source,
    },
};

#[derive(Debug)]
struct DocumentResult {
    /// CSAF tracking ID.
    tracking_id: String,
    /// Document title.
    title: String,
    /// Discovery URL.
    url: String,
    /// Profile name → mandatory check errors.
    failures: HashMap<String, Vec<CheckError>>,
    /// Profile name → optional/recommended check warnings.
    warnings: HashMap<String, Vec<CheckError>>,
    /// Profile name → informational check notes.
    infos: HashMap<String, Vec<CheckError>>,
    /// Profile names that passed.
    successes: Vec<String>,
    /// Signature/digest error message, if any.
    signature_error: Option<String>,
    /// Whether a signature file was present.
    signature_present: bool,
    /// Document category.
    category: Option<String>,
    /// Publisher name.
    publisher_name: Option<String>,
    /// Initial release date.
    initial_release_date: Option<String>,
    /// Current release date.
    current_release_date: Option<String>,
    /// Document status.
    status: Option<String>,
    /// Tracking version.
    revision: Option<String>,
    /// Aggregate severity text.
    aggregate_severity: Option<String>,
    /// CSAF specification version.
    csaf_version: Option<String>,
    /// Revision history entries.
    revision_history: Vec<RevisionEntry>,
}

/// Loads OpenPGP public keys from the provider metadata in the worktree.
async fn load_keys(file_source: &FileSource) -> Result<Vec<PublicKey>> {
    let metadata = file_source.load_metadata().await?;
    let mut keys = Vec::with_capacity(metadata.public_openpgp_keys.len());
    for key in &metadata.public_openpgp_keys {
        match file_source.load_public_key(key.into()).await {
            Ok(pk) => keys.push(pk),
            Err(e) => tracing::warn!("Failed to load public key: {e}"),
        }
    }
    Ok(keys)
}

/// Validates all documents in the worktree against basic, extended, and full CSAF profiles.
///
/// Returns the total number of documents stored for this provider after the upsert.
pub async fn validate_provider(
    state: &Arc<AppState>,
    source: &Source,
    worktree_dir: &Path,
) -> Result<u64> {
    let domain = &source.domain;
    tracing::info!("Validating documents for {domain}");

    let file_source = FileSource::new(worktree_dir, None)?;

    let keys = Arc::new(load_keys(&file_source).await.unwrap_or_else(|e| {
        tracing::warn!("Failed to load keys for {domain}: {e}");
        vec![]
    }));
    let validation_options = Arc::new(ValidationOptions::new());

    let results: Arc<Mutex<Vec<DocumentResult>>> = Arc::new(Mutex::new(Vec::new()));
    let results_ref = results.clone();

    let checks: Vec<(String, Box<dyn Check>)> = vec![
        ("basic".into(), Box::new(CsafValidation::new("basic"))),
        ("extended".into(), Box::new(CsafValidation::new("extended"))),
        ("full".into(), Box::new(CsafValidation::new("full"))),
    ];

    let state_for_closure = state.clone();
    let domain_for_closure = domain.to_string();

    let verifier = VerifyingVisitor::with_checks(
        move |result: Result<
            VerifiedAdvisory<RetrievedAdvisory, String>,
            VerificationError<_, RetrievedAdvisory>,
        >| {
            let results = results_ref.clone();
            let keys = keys.clone();
            let opts = validation_options.clone();
            let state = state_for_closure.clone();
            let domain = domain_for_closure.clone();
            async move {
                match result {
                    Ok(verified) => {
                        let tracking_id = verified.csaf.document().tracking().id().to_string();
                        let title = verified.csaf.document().title().to_string();
                        let url = verified.advisory.discovered.url.to_string();
                        let meta = extract_metadata(&verified.csaf);

                        let signature_present = verified.advisory.signature.is_some();
                        let mut sig_errors = Vec::new();

                        if let Some(sig) = &verified.advisory.signature
                            && let Err(e) =
                                validate_signature(&opts, &keys, sig, &verified.advisory.data)
                        {
                            sig_errors.push(format!("Invalid signature: {e}"));
                        }

                        if let Err((expected, actual)) = validate_digest(&verified.advisory.sha256)
                        {
                            sig_errors.push(format!(
                                "SHA-256 mismatch: expected {expected}, got {actual}"
                            ));
                        }
                        if let Err((expected, actual)) = validate_digest(&verified.advisory.sha512)
                        {
                            sig_errors.push(format!(
                                "SHA-512 mismatch: expected {expected}, got {actual}"
                            ));
                        }

                        let signature_error = if sig_errors.is_empty() {
                            None
                        } else {
                            Some(sig_errors.join("; "))
                        };

                        let failures: HashMap<String, Vec<CheckError>> = verified
                            .failures
                            .into_iter()
                            .map(|(k, v)| (k.to_string(), v))
                            .collect();
                        let warnings: HashMap<String, Vec<CheckError>> = verified
                            .warnings
                            .into_iter()
                            .map(|(k, v)| (k.to_string(), v))
                            .collect();
                        let infos: HashMap<String, Vec<CheckError>> = verified
                            .infos
                            .into_iter()
                            .map(|(k, v)| (k.to_string(), v))
                            .collect();
                        let successes: Vec<String> = verified
                            .successes
                            .into_iter()
                            .map(|s| s.to_string())
                            .collect();

                        results.lock().push(DocumentResult {
                            tracking_id,
                            title,
                            url,
                            failures,
                            warnings,
                            infos,
                            successes,
                            signature_error,
                            signature_present,
                            category: meta.category,
                            publisher_name: meta.publisher_name,
                            initial_release_date: meta.initial_release_date,
                            current_release_date: meta.current_release_date,
                            status: meta.status,
                            revision: meta.revision,
                            aggregate_severity: meta.aggregate_severity,
                            csaf_version: meta.csaf_version,
                            revision_history: meta.revision_history,
                        });
                        state.increment_job_validated(&domain).await;
                    }
                    Err(e) => {
                        let url = e.url().to_string();
                        let tracking_id = url
                            .rsplit('/')
                            .next()
                            .unwrap_or(&url)
                            .trim_end_matches(".json")
                            .to_string();
                        results.lock().push(DocumentResult {
                            tracking_id,
                            title: format!("Parse error: {e}"),
                            url,
                            failures: HashMap::new(),
                            warnings: HashMap::new(),
                            infos: HashMap::new(),
                            successes: vec![],
                            signature_error: Some(format!("Document error: {e}")),
                            signature_present: false,
                            category: None,
                            publisher_name: None,
                            initial_release_date: None,
                            current_release_date: None,
                            status: None,
                            revision: None,
                            aggregate_severity: None,
                            csaf_version: None,
                            revision_history: vec![],
                        });
                        state.increment_job_validated(&domain).await;
                    }
                }
                Ok::<_, anyhow::Error>(())
            }
        },
        checks,
    );

    let retriever = RetrievingVisitor::new(file_source.clone(), verifier);

    Walker::new(file_source)
        .walk(retriever)
        .await
        .map_err(|e| anyhow::anyhow!("Validation walker failed for {domain}: {e}"))?;

    let results = Arc::try_unwrap(results)
        .map_err(|_| anyhow::anyhow!("results Arc still shared after walk completed"))?
        .into_inner();

    let documents = build_document_results(&results);
    let total_documents = state.storage.save_documents(domain, &documents)?;

    let summary = state.storage.build_summary_from_db(domain)?;
    state.storage.save_summary(domain, &summary).await?;

    tracing::info!(
        "Validation complete for {domain}: {total_documents} documents ({} validated)",
        results.len()
    );
    Ok(total_documents)
}

/// Converts internal results into serializable document validation records.
fn build_document_results(results: &[DocumentResult]) -> Vec<DocumentValidation> {
    results
        .iter()
        .map(|doc| DocumentValidation {
            tracking_id: doc.tracking_id.clone(),
            title: doc.title.clone(),
            url: doc.url.clone(),
            profiles: DocumentProfileResults {
                basic: build_doc_profile_detail(doc, "basic"),
                extended: build_doc_profile_detail(doc, "extended"),
                full: build_doc_profile_detail(doc, "full"),
            },
            signature_error: doc.signature_error.clone(),
            signature_present: doc.signature_present,
            category: doc.category.clone(),
            publisher_name: doc.publisher_name.clone(),
            initial_release_date: doc.initial_release_date.clone(),
            current_release_date: doc.current_release_date.clone(),
            status: doc.status.clone(),
            revision: doc.revision.clone(),
            aggregate_severity: doc.aggregate_severity.clone(),
            csaf_version: doc.csaf_version.clone(),
            revision_history: doc.revision_history.clone(),
        })
        .collect()
}

/// Builds per-profile detail for a single document.
fn build_doc_profile_detail(doc: &DocumentResult, profile: &str) -> Option<DocumentProfileDetail> {
    let errors = doc.failures.get(profile);
    let warnings = doc.warnings.get(profile);
    let infos = doc.infos.get(profile);
    let has_issues = errors.is_some() || warnings.is_some() || infos.is_some();

    if has_issues {
        let mut failing_tests = Vec::new();

        if let Some(errs) = errors {
            for e in errs {
                failing_tests.push(DocumentCheckFailure {
                    test_id: e.id.to_string(),
                    message: e.message.to_string(),
                    severity: "error".to_string(),
                });
            }
        }
        if let Some(warns) = warnings {
            for w in warns {
                failing_tests.push(DocumentCheckFailure {
                    test_id: w.id.to_string(),
                    message: w.message.to_string(),
                    severity: "warning".to_string(),
                });
            }
        }
        if let Some(infs) = infos {
            for i in infs {
                failing_tests.push(DocumentCheckFailure {
                    test_id: i.id.to_string(),
                    message: i.message.to_string(),
                    severity: "info".to_string(),
                });
            }
        }

        let error_count = errors.map_or(0, |e| e.len() as u64);
        let warning_count = warnings.map_or(0, |w| w.len() as u64);
        let info_count = infos.map_or(0, |i| i.len() as u64);

        Some(DocumentProfileDetail {
            passed: false,
            error_count,
            warning_count,
            info_count,
            failing_tests,
        })
    } else if doc.successes.iter().any(|s| s == profile) {
        Some(DocumentProfileDetail {
            passed: true,
            error_count: 0,
            warning_count: 0,
            info_count: 0,
            failing_tests: vec![],
        })
    } else {
        None
    }
}

/// Extracted CSAF document metadata.
struct DocumentMetadata {
    category: Option<String>,
    publisher_name: Option<String>,
    initial_release_date: Option<String>,
    current_release_date: Option<String>,
    status: Option<String>,
    revision: Option<String>,
    aggregate_severity: Option<String>,
    csaf_version: Option<String>,
    revision_history: Vec<RevisionEntry>,
}

/// Extracts metadata fields from a parsed CSAF document.
fn extract_metadata(csaf: &Csaf) -> DocumentMetadata {
    match csaf {
        Csaf::V2_0(doc) => DocumentMetadata {
            category: Some(doc.document.category.to_string()),
            publisher_name: Some(doc.document.publisher.name.to_string()),
            initial_release_date: Some(doc.document.tracking.initial_release_date.clone()),
            current_release_date: Some(doc.document.tracking.current_release_date.clone()),
            status: Some(doc.document.tracking.status.to_string()),
            revision: Some(doc.document.tracking.version.to_string()),
            aggregate_severity: doc
                .document
                .aggregate_severity
                .as_ref()
                .map(|s| s.text.to_string()),
            csaf_version: Some("2.0".to_string()),
            revision_history: doc
                .document
                .tracking
                .revision_history
                .iter()
                .map(|r| RevisionEntry {
                    number: r.number.to_string(),
                    date: r.date.clone(),
                    summary: r.summary.to_string(),
                })
                .collect(),
        },
        Csaf::V2_1(doc) => DocumentMetadata {
            category: Some(doc.document.category.to_string()),
            publisher_name: Some(doc.document.publisher.name.to_string()),
            initial_release_date: Some(doc.document.tracking.initial_release_date.clone()),
            current_release_date: Some(doc.document.tracking.current_release_date.clone()),
            status: Some(doc.document.tracking.status.to_string()),
            revision: Some(doc.document.tracking.version.to_string()),
            aggregate_severity: doc
                .document
                .aggregate_severity
                .as_ref()
                .map(|s| s.text.to_string()),
            csaf_version: Some("2.1".to_string()),
            revision_history: doc
                .document
                .tracking
                .revision_history
                .iter()
                .map(|r| RevisionEntry {
                    number: r.number.to_string(),
                    date: r.date.clone(),
                    summary: r.summary.to_string(),
                })
                .collect(),
        },
    }
}

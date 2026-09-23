use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};

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
    source::Source as CsafSource,
    verification::{
        Csaf, VerificationError, VerifiedAdvisory, VerifyingVisitor,
        check::{Check, CsafValidation},
    },
    walker::Walker,
};
use parking_lot::Mutex;
use time::macros::datetime;
use super::source::TroveFileSource;
use crate::{
    AppState,
    models::{
        result::{
            DocumentCheckFailure, DocumentProfileDetail, DocumentProfileResults,
            DocumentValidation, RevisionEntry,
        },
        source::Source,
    },
    pipeline::sync::JobProgress,
    storage::{Storage, git_repo::document_version_counts},
};

#[derive(Debug, Clone, Copy)]
enum CsafVersionTag {
    V2_0,
    V2_1,
}

fn csaf_version_tag(csaf: &Csaf) -> CsafVersionTag {
    match csaf {
        Csaf::V2_0(_) => CsafVersionTag::V2_0,
        _ => CsafVersionTag::V2_1,
    }
}

fn total_tests_for_profile(version: CsafVersionTag, profile: &str) -> u64 {
    use csaf::validation::Validatable;
    type Csaf20 = csaf::schema::csaf2_0::schema::CommonSecurityAdvisoryFramework;
    type Csaf21 = csaf::schema::csaf2_1::schema::CommonSecurityAdvisoryFramework;
    let count = match version {
        CsafVersionTag::V2_0 => Csaf20::tests_in_preset(profile).map_or(0, |v| v.len()),
        CsafVersionTag::V2_1 => Csaf21::tests_in_preset(profile).map_or(0, |v| v.len()),
    };
    count as u64
}

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
    /// CSAF version tag for computing test counts.
    version_tag: Option<CsafVersionTag>,
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

/// Number of documents to accumulate before flushing to the database.
const VALIDATION_BATCH_SIZE: usize = 500;

/// Accumulates validation results and flushes them to the database in batches.
struct ValidationBatchState {
    /// Buffer of results awaiting flush.
    buffer: Vec<DocumentResult>,
    /// Total number of documents processed.
    total_count: u64,
}

fn build_validation_options(source: &Source) -> ValidationOptions {
    if source.accept_v3_signatures {
        ValidationOptions::new().validation_date(SystemTime::from(datetime!(2007-01-01 0:00 UTC)))
    } else {
        ValidationOptions::new()
    }
}
/// Loads OpenPGP public keys from the provider metadata in the worktree.
async fn load_keys(file_source: &TroveFileSource) -> Result<Vec<PublicKey>> {
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

    let file_source = TroveFileSource::new(worktree_dir)?;
    let canonical_worktree = Arc::new(
        std::fs::canonicalize(worktree_dir).unwrap_or_else(|_| worktree_dir.to_path_buf()),
    );

    let keys = Arc::new(load_keys(&file_source).await.unwrap_or_else(|e| {
        tracing::warn!("Failed to load keys for {domain}: {e}");
        vec![]
    }));
    let validation_options = Arc::new(build_validation_options(source));

    let db_count_before = state.storage.document_count(domain).await.unwrap_or(0);

    let batch_state: Arc<Mutex<ValidationBatchState>> =
        Arc::new(Mutex::new(ValidationBatchState {
            buffer: Vec::with_capacity(VALIDATION_BATCH_SIZE),
            total_count: 0,
        }));
    let batch_ref = batch_state.clone();

    let checks: Vec<(String, Box<dyn Check>)> = vec![
        ("basic".into(), Box::new(CsafValidation::new("basic"))),
        ("extended".into(), Box::new(CsafValidation::new("extended"))),
        ("full".into(), Box::new(CsafValidation::new("full"))),
    ];

    let state_for_closure = state.clone();
    let domain_for_closure = domain.to_string();
    let worktree_for_closure = canonical_worktree;

    let verifier = VerifyingVisitor::with_checks(
        move |result: Result<
            VerifiedAdvisory<RetrievedAdvisory, String>,
            VerificationError<_, RetrievedAdvisory>,
        >| {
            let batch = batch_ref.clone();
            let keys = keys.clone();
            let opts = validation_options.clone();
            let state = state_for_closure.clone();
            let domain = domain_for_closure.clone();
            let worktree = worktree_for_closure.clone();
            async move {
                match result {
                    Ok(verified) => {
                        let tracking_id = verified.csaf.document().tracking().id().to_string();
                        let title = verified.csaf.document().title().to_string();
                        let meta = extract_metadata(&verified.csaf);
                        let url =
                            reconstruct_original_url(&verified.advisory.discovered.url, &worktree);

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
                            .errors
                            .into_iter()
                            .map(|(k, v)| (k.to_string(), v.items))
                            .collect();
                        let warnings: HashMap<String, Vec<CheckError>> = verified
                            .warnings
                            .into_iter()
                            .map(|(k, v)| (k.to_string(), v.items))
                            .collect();
                        let infos: HashMap<String, Vec<CheckError>> = verified
                            .infos
                            .into_iter()
                            .map(|(k, v)| (k.to_string(), v.items))
                            .collect();
                        let successes: Vec<String> = verified
                            .successes
                            .into_iter()
                            .map(|s| s.to_string())
                            .collect();

                        let vtag = Some(csaf_version_tag(&verified.csaf));

                        let batch_to_flush = {
                            let mut b = batch.lock();
                            b.buffer.push(DocumentResult {
                                tracking_id,
                                title,
                                url,
                                failures,
                                warnings,
                                infos,
                                successes,
                                version_tag: vtag,
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
                            b.total_count += 1;
                            if b.buffer.len() >= VALIDATION_BATCH_SIZE {
                                Some(std::mem::replace(
                                    &mut b.buffer,
                                    Vec::with_capacity(VALIDATION_BATCH_SIZE),
                                ))
                            } else {
                                None
                            }
                        };

                        if let Some(drained) = batch_to_flush {
                            flush_batch(&state.storage, &domain, drained).await?;
                        }

                        state.increment_job_validated(&domain).await;
                    }
                    Err(e) => {
                        let url = reconstruct_original_url(e.url(), &worktree);
                        let tracking_id = url
                            .rsplit('/')
                            .next()
                            .unwrap_or(&url)
                            .trim_end_matches(".json")
                            .to_string();
                        let batch_to_flush = {
                            let mut b = batch.lock();
                            b.buffer.push(DocumentResult {
                                tracking_id,
                                title: format!("Parse error: {e}"),
                                url,
                                failures: HashMap::new(),
                                warnings: HashMap::new(),
                                infos: HashMap::new(),
                                successes: vec![],
                                version_tag: None,
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
                            b.total_count += 1;
                            if b.buffer.len() >= VALIDATION_BATCH_SIZE {
                                Some(std::mem::replace(
                                    &mut b.buffer,
                                    Vec::with_capacity(VALIDATION_BATCH_SIZE),
                                ))
                            } else {
                                None
                            }
                        };

                        if let Some(drained) = batch_to_flush {
                            flush_batch(&state.storage, &domain, drained).await?;
                        }

                        state.increment_job_validated(&domain).await;
                    }
                }
                Ok::<_, anyhow::Error>(())
            }
        },
        checks,
    );

    let retriever = RetrievingVisitor::new(file_source.clone(), verifier);
    let distributions_total = Arc::new(AtomicU64::new(0));
    let dt = distributions_total.clone();

    Walker::new(file_source)
        .with_distribution_filter(move |_| {
            dt.fetch_add(1, Ordering::Relaxed);
            true
        })
        .with_progress(JobProgress {
            state: state.clone(),
            domain: domain.to_string(),
            distributions_total,
        })
        .walk(retriever)
        .await
        .map_err(|e| anyhow::anyhow!("Validation walker failed for {domain}: {e}"))?;

    let (remaining, total_count) = {
        let mut b = batch_state.lock();
        (std::mem::take(&mut b.buffer), b.total_count)
    };

    if db_count_before > 0 && total_count < db_count_before / 2 {
        tracing::warn!(
            "{domain}: validation found {total_count} documents but database has {db_count_before}; \
             documents not in this batch are preserved via upsert",
        );
    }

    flush_batch(&state.storage, domain, remaining).await?;

    let total_documents = state.storage.document_count(domain).await?;

    let summary = state.storage.build_summary_from_db(domain).await?;
    state.storage.save_summary(domain, &summary).await?;

    tracing::info!(
        "Validation complete for {domain}: {total_documents} documents ({total_count} validated)",
    );
    Ok(total_documents)
}

/// Converts a batch of results to `DocumentValidation` and writes them to the database.
async fn flush_batch(storage: &Storage, domain: &str, batch: Vec<DocumentResult>) -> Result<()> {
    if batch.is_empty() {
        return Ok(());
    }
    let mut documents = build_document_results(batch);
    // Compute version counts from git history
    let repo_path = storage.repo_path(domain);
    if repo_path.exists() {
        let urls: Vec<String> = documents.iter().map(|d| d.url.clone()).collect();
        let counts = tokio::task::spawn_blocking(move || {
            let url_refs: Vec<&str> = urls.iter().map(|s| s.as_str()).collect();
            document_version_counts(&repo_path, &url_refs)
        })
        .await??;
        for doc in &mut documents {
            doc.version_count = counts.get(&doc.url).copied().unwrap_or(1);
        }
    }
    storage.save_documents(domain, &documents).await?;
    Ok(())
}

/// Converts internal results into serializable document validation records.
fn build_document_results(results: Vec<DocumentResult>) -> Vec<DocumentValidation> {
    results
        .into_iter()
        .map(|doc| {
            let profiles = DocumentProfileResults {
                basic: build_doc_profile_detail(&doc, "basic"),
                extended: build_doc_profile_detail(&doc, "extended"),
                full: build_doc_profile_detail(&doc, "full"),
            };
            DocumentValidation {
                tracking_id: doc.tracking_id,
                title: doc.title,
                url: doc.url,
                profiles,
                signature_error: doc.signature_error,
                signature_present: doc.signature_present,
                category: doc.category,
                publisher_name: doc.publisher_name,
                initial_release_date: doc.initial_release_date,
                current_release_date: doc.current_release_date,
                status: doc.status,
                revision: doc.revision,
                aggregate_severity: doc.aggregate_severity,
                csaf_version: doc.csaf_version,
                revision_history: doc.revision_history,
                version_count: 1,
                retrieval_error: None,
            }
        })
        .collect()
}

/// Builds per-profile detail for a single document.
fn build_doc_profile_detail(doc: &DocumentResult, profile: &str) -> Option<DocumentProfileDetail> {
    let errors = doc.failures.get(profile);
    let warnings = doc.warnings.get(profile);
    let infos = doc.infos.get(profile);
    let has_issues = errors.is_some() || warnings.is_some() || infos.is_some();

    let total_tests = doc
        .version_tag
        .map(|v| total_tests_for_profile(v, profile))
        .unwrap_or(0);

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

        let distinct_failing: HashSet<&str> =
            failing_tests.iter().map(|f| f.test_id.as_str()).collect();
        let failing_test_count = distinct_failing.len() as u64;

        let error_count = errors.map_or(0, |e| e.len() as u64);
        let warning_count = warnings.map_or(0, |w| w.len() as u64);
        let info_count = infos.map_or(0, |i| i.len() as u64);

        Some(DocumentProfileDetail {
            passed: false,
            error_count,
            warning_count,
            info_count,
            total_tests,
            failing_test_count,
            failing_tests,
        })
    } else if doc.successes.iter().any(|s| s == profile) {
        Some(DocumentProfileDetail {
            passed: true,
            error_count: 0,
            warning_count: 0,
            info_count: 0,
            total_tests,
            failing_test_count: 0,
            failing_tests: vec![],
        })
    } else {
        None
    }
}

/// Reconstructs the original HTTP(S) URL from a `file://` URL produced by [`FileSource`].
///
/// The worktree stores files under a percent-encoded distribution URL directory.
/// Falls back to the file URL string if reconstruction fails.
fn reconstruct_original_url(file_url: &url::Url, worktree_dir: &Path) -> String {
    try_reconstruct_url(file_url, worktree_dir).unwrap_or_else(|| file_url.to_string())
}

/// Reconstructs the original HTTPS URL from a `file://` URL in the worktree.
///
/// The worktree stores files as `<domain>/<url_path>`. The first path component
/// after the worktree root is the domain name; the rest is the URL path.
fn try_reconstruct_url(file_url: &url::Url, worktree_dir: &Path) -> Option<String> {
    let path = file_url.to_file_path().ok()?;
    let relative = path.strip_prefix(worktree_dir).ok()?;

    let mut components = relative.components();
    let domain = components.next()?.as_os_str().to_str()?;

    if domain == "metadata" {
        return None;
    }

    let remaining: PathBuf = components.collect();
    let url_path = remaining.to_str()?;

    if url_path.is_empty() {
        return None;
    }

    Some(format!("https://{domain}/{url_path}"))
}

/// Extracted CSAF document metadata.
#[derive(Default)]
struct DocumentMetadata {
    /// Document category.
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
        _ => DocumentMetadata::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconstructs_https_url_from_file_url() {
        let worktree = PathBuf::from("/tmp/work/provider");
        let file_path = "/tmp/work/provider/security.access.redhat.com/data/csaf/v2/advisories/2024/rhsa-2024_1234.json";
        let file_url = url::Url::from_file_path(file_path).unwrap();

        let result = try_reconstruct_url(&file_url, &worktree);
        assert_eq!(
            result,
            Some("https://security.access.redhat.com/data/csaf/v2/advisories/2024/rhsa-2024_1234.json".to_string())
        );
    }

    #[test]
    fn skips_metadata_directory() {
        let worktree = PathBuf::from("/tmp/work/provider");
        let file_path = "/tmp/work/provider/metadata/provider-metadata.json";
        let file_url = url::Url::from_file_path(file_path).unwrap();

        assert_eq!(try_reconstruct_url(&file_url, &worktree), None);
    }

    #[test]
    fn falls_back_on_wrong_prefix() {
        let worktree = PathBuf::from("/tmp/other");
        let file_url = url::Url::from_file_path("/tmp/work/example.com/doc.json").unwrap();

        assert_eq!(try_reconstruct_url(&file_url, &worktree), None);
    }

    #[test]
    fn falls_back_on_non_file_url() {
        let worktree = PathBuf::from("/tmp/work");
        let url = url::Url::parse("https://example.com/doc.json").unwrap();

        assert_eq!(try_reconstruct_url(&url, &worktree), None);
    }

    #[test]
    fn falls_back_on_domain_only() {
        let worktree = PathBuf::from("/tmp/work");
        let file_path = "/tmp/work/example.com";
        let file_url = url::Url::from_file_path(file_path).unwrap();

        assert_eq!(try_reconstruct_url(&file_url, &worktree), None);
    }

    #[test]
    fn reconstruct_original_url_returns_file_url_on_failure() {
        let worktree = PathBuf::from("/tmp/other");
        let url = url::Url::parse("https://example.com/doc.json").unwrap();

        let result = reconstruct_original_url(&url, &worktree);
        assert_eq!(result, "https://example.com/doc.json");
    }
}

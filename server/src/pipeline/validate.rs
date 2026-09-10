use std::{collections::HashMap, path::Path, sync::Arc};

use anyhow::Result;
use csaf_walker::{
    check::CheckError,
    retrieve::{RetrievedAdvisory, RetrievingVisitor},
    source::FileSource,
    verification::{
        VerifiedAdvisory, VerifyingVisitor,
        check::{Check, CsafValidation},
    },
    walker::Walker,
};
use parking_lot::Mutex;

use crate::{
    AppState,
    models::{
        result::{FailingTest, ProfileResults, ProfileSummary, ProviderSummary},
        source::Source,
    },
};

#[derive(Debug)]
struct DocumentResult {
    failures: HashMap<String, Vec<CheckError>>,
    successes: Vec<String>,
}

/// Validates all documents in the worktree against basic, extended, and full CSAF profiles.
pub async fn validate_provider(
    state: &Arc<AppState>,
    source: &Source,
    worktree_dir: &Path,
) -> Result<()> {
    let domain = &source.domain;
    tracing::info!("Validating documents for {domain}");

    let file_source = FileSource::new(worktree_dir, None)?;

    let results: Arc<Mutex<Vec<DocumentResult>>> = Arc::new(Mutex::new(Vec::new()));
    let results_ref = results.clone();

    let checks: Vec<(String, Box<dyn Check>)> = vec![
        ("basic".into(), Box::new(CsafValidation::new("basic"))),
        ("extended".into(), Box::new(CsafValidation::new("extended"))),
        ("full".into(), Box::new(CsafValidation::new("full"))),
    ];

    let verifier = VerifyingVisitor::with_checks(
        move |result: Result<VerifiedAdvisory<RetrievedAdvisory, String>, _>| {
            let results = results_ref.clone();
            async move {
                if let Ok(verified) = result {
                    let failures: HashMap<String, Vec<CheckError>> = verified
                        .failures
                        .into_iter()
                        .map(|(k, v)| (k.to_string(), v))
                        .collect();
                    let successes: Vec<String> = verified
                        .successes
                        .into_iter()
                        .map(|s| s.to_string())
                        .collect();
                    results.lock().push(DocumentResult {
                        failures,
                        successes,
                    });
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

    let summary = build_summary(domain, &results);
    state.storage.save_summary(domain, &summary).await?;

    tracing::info!(
        "Validation complete for {domain}: {} documents",
        results.len()
    );
    Ok(())
}

fn build_summary(domain: &str, results: &[DocumentResult]) -> ProviderSummary {
    let document_count = results.len() as u64;

    let basic = build_profile_summary(results, "basic");
    let extended = build_profile_summary(results, "extended");
    let full = build_profile_summary(results, "full");

    let mut test_counts: HashMap<String, u64> = HashMap::new();
    for doc in results {
        for errors in doc.failures.values() {
            for error in errors {
                *test_counts.entry(error.id.to_string()).or_default() += 1;
            }
        }
    }

    let mut top_failing: Vec<_> = test_counts.into_iter().collect();
    top_failing.sort_by_key(|a| std::cmp::Reverse(a.1));
    top_failing.truncate(10);

    ProviderSummary {
        provider: domain.to_string(),
        publisher_name: None,
        validated_at: chrono::Utc::now(),
        document_count,
        profiles: ProfileResults {
            basic: Some(basic),
            extended: Some(extended),
            full: Some(full),
        },
        top_failing_tests: top_failing
            .into_iter()
            .map(|(test_id, count)| FailingTest {
                test_id,
                count,
                severity: "error".to_string(),
            })
            .collect(),
    }
}

fn build_profile_summary(results: &[DocumentResult], profile: &str) -> ProfileSummary {
    let mut valid = 0u64;
    let mut invalid = 0u64;

    for doc in results {
        if doc.failures.contains_key(profile) {
            invalid += 1;
        } else if doc.successes.iter().any(|s| s == profile) {
            valid += 1;
        }
    }

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

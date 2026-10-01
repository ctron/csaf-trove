use crate::{
    AppState,
    models::{
        source::{Source, sanitize_domain},
        state::{JobPhase, JobStatus},
    },
    pipeline::{
        report::generate_report,
        store::DIR_METADATA,
        sync::{RetrievalFailure, sync_provider},
        validate::validate_provider,
    },
    storage::{
        ProviderInfo,
        git_processing::{
            include_recovered_downloads, materialize_processing_with_progress,
            select_processing_with_progress,
        },
        git_repo,
    },
};
use anyhow::{Error, Result};
use csaf_trove_common::PipelinePhase;
use csaf_walker::model::metadata::{ProviderMetadata, Role};
use parking_lot::Mutex;
use std::{
    collections::VecDeque,
    fs,
    future::Future,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use time::OffsetDateTime;
use tokio::{pin, runtime::Handle, select, task::spawn_blocking, time::sleep};

mod validation_identity;

use validation_identity::dependency_fingerprint;

/// One coherent progress sample from blocking work.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct ProgressSample {
    /// Optional phase transition reported by blocking work.
    phase: Option<PipelinePhase>,
    /// Work completed in this phase.
    current: u64,
    /// Expected work, or zero when unknown.
    total: u64,
}

/// Bridges blocking work to async status updates at most four times per second.
struct BlockingProgress {
    /// Pending phase transitions with the latest counters for each phase.
    samples: Arc<Mutex<VecDeque<ProgressSample>>>,
}

impl BlockingProgress {
    /// Creates an empty progress bridge.
    fn new() -> Self {
        Self {
            samples: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Returns a callback for work whose phase has already been published.
    fn callback(&self) -> impl Fn(u64, u64) + Send + Sync + 'static {
        let samples = self.samples.clone();
        move |current, total| {
            Self::record(
                &samples,
                ProgressSample {
                    phase: None,
                    current,
                    total,
                },
            );
        }
    }

    /// Returns a callback that publishes phase changes with consistent counters.
    fn stage_callback(&self) -> impl Fn(PipelinePhase, u64, u64) + Send + Sync + 'static {
        let samples = self.samples.clone();
        move |phase, current, total| {
            Self::record(
                &samples,
                ProgressSample {
                    phase: Some(phase),
                    current,
                    total,
                },
            );
        }
    }

    /// Coalesces counter updates while preserving every phase transition.
    fn record(samples: &Mutex<VecDeque<ProgressSample>>, next: ProgressSample) {
        let mut samples = samples.lock();
        if let Some(last) = samples.back_mut().filter(|last| last.phase == next.phase) {
            *last = next;
        } else {
            samples.push_back(next);
        }
    }

    /// Publishes queued transitions and progress without holding the blocking mutex.
    async fn publish(&self, state: &Arc<AppState>, domain: &str) {
        let pending: Vec<_> = self.samples.lock().drain(..).collect();
        for sample in pending {
            state
                .set_phase_work(domain, sample.phase, sample.current, sample.total)
                .await;
        }
    }

    /// Polls blocking progress, then publishes the final sample even for unknown totals.
    async fn run<F: Future<Output = T>, T>(
        &self,
        state: &Arc<AppState>,
        domain: &str,
        future: F,
    ) -> T {
        pin!(future);
        loop {
            select! {
                result = &mut future => {
                    self.publish(state, domain).await;
                    return result;
                }
                _ = sleep(Duration::from_millis(250)) => self.publish(state, domain).await,
            }
        }
    }
}

/// Kind of work a tracked provider job performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JobKind {
    /// Full pipeline: sync, commit, validate, report.
    Sync,
    /// Re-validate already stored documents without fetching anything.
    Revalidate,
}

/// Runs the full pipeline (sync, validate, report) for a provider with job status tracking.
///
/// Acquires a per-provider lock so concurrent runs for the same domain are skipped.
pub async fn run_provider(state: &Arc<AppState>, source: &Source) -> Result<()> {
    run_job(state, source, JobKind::Sync).await
}

/// Re-validates all stored documents of a provider with job status tracking.
///
/// Shares the per-provider lock with [`run_provider`], so it is skipped while a sync runs.
pub async fn revalidate_provider(state: &Arc<AppState>, source: &Source) -> Result<()> {
    run_job(state, source, JobKind::Revalidate).await
}

/// Runs a provider job of the given kind, tracking its status and holding the provider lock.
async fn run_job(state: &Arc<AppState>, source: &Source, kind: JobKind) -> Result<()> {
    let domain = &source.domain;

    let lock = {
        let mut locks = state.pipeline_locks.write().await;
        locks.entry(domain.to_string()).or_default().clone()
    };
    let Ok(_guard) = lock.try_lock() else {
        tracing::warn!("Pipeline for {domain} already running, skipping");
        return Ok(());
    };

    tracing::info!("Starting {kind:?} pipeline for {domain}");

    let last_completed = state.get_job(domain).await.and_then(|j| j.completed_at);
    let now = OffsetDateTime::now_utc();

    let job = JobStatus {
        status: JobPhase::Running,
        started_at: now,
        completed_at: None,
        phase: Some(PipelinePhase::Checkout),
        documents_synced: 0,
        documents_validated: 0,
        documents_total: 0,
        distributions_total: 0,
        distribution_index: 0,
        distribution_documents_current: 0,
        distribution_documents_total: 0,
        phase_current: 0,
        phase_total: 0,
        error: None,
        completed_phases: vec![],
        last_completed_at: last_completed,
        phase_started_at: Some(now),
    };
    state.update_job(domain, job).await;

    let result = match kind {
        JobKind::Sync => run_pipeline(state, source).await,
        JobKind::Revalidate => run_revalidation(state, source).await,
    };

    match &result {
        Ok(()) => {
            let mut job = state.get_job(domain).await.unwrap_or_else(|| JobStatus {
                status: JobPhase::Completed,
                started_at: OffsetDateTime::now_utc(),
                completed_at: Some(OffsetDateTime::now_utc()),
                phase: None,
                documents_synced: 0,
                documents_validated: 0,
                documents_total: 0,
                distributions_total: 0,
                distribution_index: 0,
                distribution_documents_current: 0,
                distribution_documents_total: 0,
                phase_current: 0,
                phase_total: 0,
                error: None,
                completed_phases: vec![],
                last_completed_at: None,
                phase_started_at: None,
            });
            job.status = JobPhase::Completed;
            job.completed_at = Some(OffsetDateTime::now_utc());
            if let Some(last) = job.phase.take() {
                job.completed_phases.push(last);
            }
            state.update_job(domain, job.clone()).await;
            persist_job_counts(state, domain, &job).await;
            tracing::info!("Pipeline for {domain} completed");
        }
        Err(e) => {
            let mut job = state.get_job(domain).await.unwrap_or_else(|| JobStatus {
                status: JobPhase::Failed,
                started_at: OffsetDateTime::now_utc(),
                completed_at: Some(OffsetDateTime::now_utc()),
                phase: None,
                documents_synced: 0,
                documents_validated: 0,
                documents_total: 0,
                distributions_total: 0,
                distribution_index: 0,
                distribution_documents_current: 0,
                distribution_documents_total: 0,
                phase_current: 0,
                phase_total: 0,
                error: None,
                completed_phases: vec![],
                last_completed_at: None,
                phase_started_at: None,
            });
            job.status = JobPhase::Failed;
            job.completed_at = Some(OffsetDateTime::now_utc());
            job.error = Some(format!("{e:#}"));
            state.update_job(domain, job).await;
            tracing::error!("Pipeline for {domain} failed: {e:#}");
        }
    }

    result
}

/// Saves the final document counts from a completed job into the persisted sync state
/// and records the sync run in the database.
async fn persist_job_counts(state: &Arc<AppState>, domain: &str, job: &JobStatus) {
    match state.storage.load_sync_state(domain).await {
        Ok(mut sync_state) => {
            sync_state.documents_synced = job.documents_synced;
            sync_state.documents_validated = job.documents_validated;
            sync_state.documents_total = job.documents_total;
            if let Err(e) = state.storage.save_sync_state(&sync_state).await {
                tracing::warn!("Failed to persist job counts for {domain}: {e}");
            }
        }
        Err(e) => {
            tracing::warn!("Failed to load sync state for {domain}: {e}");
        }
    }

    let now = OffsetDateTime::now_utc();
    let duration = job.completed_at.unwrap_or(now) - job.started_at;
    if let Err(e) = state
        .storage
        .save_sync_run(domain, now, job.documents_synced, duration)
        .await
    {
        tracing::warn!("Failed to save sync run for {domain}: {e}");
    }

    if let Ok(Some(points)) = state.storage.recent_sync_points(domain, 30).await {
        state
            .recent_sync_points
            .write()
            .await
            .insert(domain.to_string(), points);
    }
}

/// Runs a provider sync and removes its scratch directory on success or failure.
async fn run_pipeline(state: &Arc<AppState>, source: &Source) -> Result<()> {
    let domain = &source.domain;
    let repo_path = state.storage.repo_path(domain);
    let worktree_dir = state.work_dir().join(sanitize_domain(domain));

    let result = async {
        let prepared = {
            let repo = repo_path.clone();
            let worktree = worktree_dir.clone();
            spawn_blocking(move || git_repo::prepare_worktree(&repo, &worktree)).await??
        };

        state
            .update_job_phase(domain, PipelinePhase::Discover)
            .await;

        let sync_result = match sync_provider(state, source, &worktree_dir).await {
            Ok(result) => result,
            Err(e) => {
                tracing::error!("Sync failed for {domain}: {e:#}");
                return Err(e);
            }
        };

        state.update_job_phase(domain, PipelinePhase::Commit).await;
        let committed = {
            let now = OffsetDateTime::now_utc();
            let msg = format!(
                "sync: {:04}-{:02}-{:02}T{:02}:{:02}Z",
                now.year(),
                now.month() as u8,
                now.day(),
                now.hour(),
                now.minute(),
            );
            let pack_threshold = state.config.data.git_pack_threshold.0;
            let progress = BlockingProgress::new();
            let cb = progress.callback();
            progress
                .run(state, domain, async {
                    spawn_blocking(move || {
                        git_repo::commit_snapshot_with_progress(&prepared, &msg, pack_threshold, cb)
                    })
                    .await?
                })
                .await?
        };
        tracing::debug!(commit = %committed.commit_id, "Committed provider snapshot");

        persist_provider_metadata(state, domain, &worktree_dir).await;

        state
            .storage
            .save_distribution_errors(domain, &sync_result.distribution_errors)
            .await?;
        process_snapshot(
            state,
            source,
            &worktree_dir,
            false,
            &sync_result.retrieval_errors,
        )
        .await?;

        state
            .storage
            .save_distribution_membership(domain, &sync_result.membership)
            .await?;

        // only advance the since token once the full run succeeded, so a failure in a later
        // phase causes the affected documents to be processed again on the next run
        let mut sync_state = state.storage.load_sync_state(domain).await?;
        sync_state.last_sync = Some(OffsetDateTime::now_utc());
        sync_state.since_token = Some(sync_result.started_at);
        state.storage.save_sync_state(&sync_state).await?;

        Ok(())
    }
    .await;
    cleanup_worktree(&worktree_dir).await;
    result
}

/// Materializes and re-validates every advisory in the stored repository.
///
/// Nothing is fetched from the provider; existing results are updated in place.
async fn run_revalidation(state: &Arc<AppState>, source: &Source) -> Result<()> {
    let domain = &source.domain;
    let worktree_dir = state.work_dir().join(sanitize_domain(domain));

    // No checkout is needed: processing materializes the selected committed inputs.
    let result = process_snapshot(state, source, &worktree_dir, true, &[]).await;
    cleanup_worktree(&worktree_dir).await;
    result
}

/// Identifies local validation code, dependency versions and effective signature policy.
fn validator_identity(source: &Source) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(include_bytes!("../pipeline/validate.rs"));
    hash.update(include_bytes!("../pipeline/source.rs"));
    /// Bump when persisted results must be rebuilt after a format or storage change.
    const RESULT_FORMAT_REVISION: u32 = 1;
    hash.update(RESULT_FORMAT_REVISION.to_le_bytes());
    hash.update(dependency_fingerprint(include_str!("../../../Cargo.lock"))?);
    hash.update([u8::from(source.accept_v3_signatures)]);
    Ok(hex::encode(hash.finalize()))
}

/// Logs elapsed time and outcome even when a processing step returns an error.
async fn timed_processing_step<T>(
    domain: &str,
    step: &'static str,
    work: impl Future<Output = Result<T>>,
) -> Result<T> {
    let started = Instant::now();
    tracing::info!(domain, step, "Processing step started");
    let result = work.await;
    let elapsed_ms = started.elapsed().as_millis();
    match &result {
        Ok(_) => tracing::info!(
            domain,
            step,
            elapsed_ms,
            success = true,
            "Processing step finished"
        ),
        Err(error) => {
            tracing::warn!(domain, step, elapsed_ms, success = false, error = %error, "Processing step finished")
        }
    }
    result
}

/// Processes only inputs changed since the last successful snapshot, then publishes its checkpoint.
async fn process_snapshot(
    state: &Arc<AppState>,
    source: &Source,
    download_dir: &Path,
    force_full: bool,
    retrieval_errors: &[RetrievalFailure],
) -> Result<()> {
    let started = Instant::now();
    let domain = &source.domain;
    let previous = state.storage.processing_checkpoint(domain).await?;
    let summary_dirty = previous
        .as_ref()
        .is_some_and(|checkpoint| checkpoint.summary_dirty);
    let previous_commit = previous
        .as_ref()
        .map(|checkpoint| checkpoint.commit_id.clone());
    if force_full {
        state.storage.clear_processing_checkpoint(domain).await?;
    }
    let repo = state.storage.repo_path(domain);
    let identity = validator_identity(source)?;
    let selection_repo = repo.clone();
    state
        .update_job_phase(domain, PipelinePhase::CleanSelection)
        .await;
    let selection_directory = download_dir.join(".selection");
    let directory = selection_directory.clone();
    spawn_blocking(move || {
        if directory.exists() {
            fs::remove_dir_all(&directory)?;
        }
        fs::create_dir_all(&directory)
    })
    .await??;
    state
        .update_job_phase(domain, PipelinePhase::SelectInputs)
        .await;
    let progress = BlockingProgress::new();
    let cb = progress.stage_callback();
    let runtime = Handle::current();
    let mut selection = timed_processing_step(
        domain,
        "selection",
        progress.run(state, domain, async {
            spawn_blocking(move || {
                runtime.block_on(select_processing_with_progress(
                    &selection_repo,
                    &selection_directory,
                    previous.as_ref(),
                    &identity,
                    force_full,
                    cb,
                ))
            })
            .await?
        }),
    )
    .await?;
    state
        .update_job_phase(domain, PipelinePhase::CheckRecoveredDownloads)
        .await;
    if !selection.full {
        let mut cursor = String::new();
        let mut checked = 0;
        loop {
            let failed_urls = state.storage.retrieval_error_urls(domain, &cursor).await?;
            if failed_urls.is_empty() {
                break;
            }
            include_recovered_downloads(&mut selection, &failed_urls, download_dir).await?;
            checked += failed_urls.len() as u64;
            state.set_phase_progress(domain, checked, 0).await;
            if let Some(last) = failed_urls.last() {
                cursor.clone_from(last);
            }
        }
    }
    let validation_dir = download_dir.join("validation");
    let work = validation_dir.clone();
    state
        .update_job_phase(domain, PipelinePhase::CleanValidationFiles)
        .await;
    timed_processing_step(domain, "cleanup", async {
        spawn_blocking(move || {
            if work.exists() {
                fs::remove_dir_all(&work)?;
            }
            Ok::<_, Error>(())
        })
        .await?
    })
    .await?;
    state
        .update_job_phase(domain, PipelinePhase::ExtractValidationFiles)
        .await;
    let work = validation_dir.clone();
    let progress = BlockingProgress::new();
    let cb = progress.callback();
    let runtime = Handle::current();
    let (selection, selected_count) = timed_processing_step(
        domain,
        "materialization",
        progress.run(state, domain, async {
            spawn_blocking(move || {
                runtime.block_on(async {
                    let count = if selection.full || !selection.advisories.is_empty() {
                        materialize_processing_with_progress(&repo, &selection, &work, cb).await?
                    } else {
                        0
                    };
                    Ok::<_, Error>((selection, count))
                })
            })
            .await?
        }),
    )
    .await?;
    let mut changed =
        summary_dirty || selection.full || selected_count > 0 || !selection.deleted.is_empty();
    state
        .update_job_phase(domain, PipelinePhase::CleanResults)
        .await;
    if selection.baseline {
        state.storage.delete_all_documents(domain).await?;
    } else {
        let mut cursor = String::new();
        let mut deleted = 0;
        loop {
            let urls = selection.deleted.batch(&cursor).await?;
            if urls.is_empty() {
                break;
            }
            state.storage.delete_document_urls(domain, &urls).await?;
            deleted += urls.len() as u64;
            state
                .set_phase_progress(domain, deleted, selection.deleted.len() as u64)
                .await;
            if let Some(last) = urls.last() {
                cursor.clone_from(last);
            }
        }
    }
    if selected_count > 0 {
        state
            .update_job_phase(domain, PipelinePhase::Validate)
            .await;
        timed_processing_step(
            domain,
            "validation",
            validate_provider(state, source, &validation_dir, selected_count),
        )
        .await?;
    }
    // Documents stored before versions were recorded need a one-time full history scan.
    let backfill = !selection.baseline && state.storage.versions_missing(domain).await?;
    if selection.baseline || backfill || !selection.history.is_empty() {
        state
            .update_job_phase(domain, PipelinePhase::VersionCounts)
            .await;
        let since = if selection.baseline || backfill {
            None
        } else {
            previous_commit.as_deref()
        };
        timed_processing_step(
            domain,
            "version_recording",
            state.storage.record_versions(domain, since),
        )
        .await?;
    }
    if !retrieval_errors.is_empty() {
        let pairs = retrieval_errors
            .iter()
            .map(|failure| (failure.url.clone(), failure.error.clone()))
            .collect::<Vec<_>>();
        changed |= state.storage.save_retrieval_errors(domain, &pairs).await?;
    }
    if changed || state.storage.load_summary(domain).await?.is_none() {
        state.update_job_phase(domain, PipelinePhase::Summary).await;
        timed_processing_step(domain, "summary", async {
            let summary = state.storage.build_summary_from_db(domain).await?;
            state.storage.save_summary(domain, &summary).await
        })
        .await?;
    }
    state.update_job_phase(domain, PipelinePhase::Report).await;
    timed_processing_step(domain, "report", generate_report(state, source)).await?;
    let total = state.storage.document_count(domain).await?;
    if let Some(job) = state.jobs.write().await.get_mut(domain) {
        job.documents_total = total;
    }
    state
        .storage
        .save_processing_checkpoint(domain, &selection.checkpoint)
        .await?;
    tracing::info!(
        domain,
        full = selection.full,
        selected = selected_count,
        skipped = total.saturating_sub(selected_count),
        elapsed_ms = started.elapsed().as_millis(),
        "Provider processing complete"
    );
    Ok(())
}

/// Removes disposable files without touching the persistent object database.
async fn cleanup_worktree(worktree_dir: &PathBuf) {
    if worktree_dir.exists()
        && let Err(e) = tokio::fs::remove_dir_all(worktree_dir).await
    {
        tracing::warn!(
            "Failed to clean up worktree {}: {e}",
            worktree_dir.display()
        );
    }
}

/// Reads provider-metadata.json from the worktree and persists aggregator-relevant fields.
async fn persist_provider_metadata(state: &Arc<AppState>, domain: &str, worktree_dir: &Path) {
    let metadata_path = worktree_dir
        .join(DIR_METADATA)
        .join("provider-metadata.json");
    let data = match tokio::fs::read_to_string(&metadata_path).await {
        Ok(d) => d,
        Err(_) => return,
    };

    let metadata: ProviderMetadata = match serde_json::from_str(&data) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("{domain}: failed to parse provider-metadata.json: {e}");
            return;
        }
    };

    let role_str = match metadata.role {
        Role::Publisher => "csaf_publisher",
        Role::Provider => "csaf_provider",
        Role::TrustedProvider => "csaf_trusted_provider",
    };

    let info = ProviderInfo {
        canonical_url: metadata.canonical_url.to_string(),
        publisher_name: metadata.publisher.name.clone(),
        publisher_category: serde_json::to_value(&metadata.publisher.category)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "other".to_string()),
        publisher_namespace: metadata.publisher.namespace.clone(),
        role: Some(role_str.to_string()),
        list_on_aggregators: metadata.list_on_csaf_aggregators,
        mirror_on_aggregators: metadata.mirror_on_csaf_aggregators,
        last_updated: metadata.last_updated.to_rfc3339(),
    };

    if let Err(e) = state.storage.save_provider_info(domain, &info).await {
        tracing::warn!("{domain}: failed to persist provider info: {e}");
    }
}

#[cfg(test)]
mod tests;

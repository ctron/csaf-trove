use crate::{
    AppState,
    models::{
        source::{Source, sanitize_domain},
        state::{JobPhase, JobStatus},
    },
    pipeline::{
        report::generate_report, store::DIR_METADATA, sync::sync_provider,
        validate::validate_provider,
    },
    storage::{ProviderInfo, git_repo},
};
use anyhow::Result;
use csaf_trove_common::PipelinePhase;
use csaf_walker::model::metadata::{ProviderMetadata, Role};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use time::OffsetDateTime;

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
    if let Err(e) = state
        .storage
        .save_sync_run(domain, now, job.documents_synced)
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
        let sync_state = state.storage.load_sync_state(domain).await?;
        let db_count = state.storage.document_count(domain).await?;
        let incremental = sync_state.since_token.is_some() && db_count > 0;

        if incremental {
            tracing::info!(
                "{domain}: incremental mode — skipping worktree checkout ({db_count} docs in DB)"
            );
        }

        let prepared = {
            let repo = repo_path.clone();
            let worktree = worktree_dir.clone();
            tokio::task::spawn_blocking(move || {
                git_repo::prepare_worktree(&repo, &worktree, incremental)
            })
            .await??
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
        {
            let now = OffsetDateTime::now_utc();
            let msg = format!(
                "sync: {:04}-{:02}-{:02}T{:02}:{:02}Z",
                now.year(),
                now.month() as u8,
                now.day(),
                now.hour(),
                now.minute(),
            );
            tokio::task::spawn_blocking(move || git_repo::commit_all(&prepared, &msg)).await??;
        }

        persist_provider_metadata(state, domain, &worktree_dir).await;

        if !incremental {
            state.storage.delete_all_documents(domain).await?;
        }

        state
            .update_job_phase(domain, PipelinePhase::Validate)
            .await;
        let total = validate_provider(state, source, &worktree_dir).await?;

        {
            let mut jobs = state.jobs.write().await;
            if let Some(job) = jobs.get_mut(domain) {
                job.documents_total = total;
            }
        }

        if !sync_result.retrieval_errors.is_empty() {
            let pairs: Vec<(String, String)> = sync_result
                .retrieval_errors
                .into_iter()
                .map(|e| (e.url, e.error))
                .collect();
            state.storage.save_retrieval_errors(domain, &pairs).await?;
        }

        state
            .storage
            .save_distribution_errors(domain, &sync_result.distribution_errors)
            .await?;

        state.update_job_phase(domain, PipelinePhase::Report).await;
        generate_report(state, source).await?;

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

/// Checks out the stored repository and re-validates every document in it.
///
/// Nothing is fetched from the provider; existing results are updated in place.
async fn run_revalidation(state: &Arc<AppState>, source: &Source) -> Result<()> {
    let domain = &source.domain;
    let repo_path = state.storage.repo_path(domain);
    let worktree_dir = state.work_dir().join(sanitize_domain(domain));

    let result = async {
        {
            let repo = repo_path.clone();
            let worktree = worktree_dir.clone();
            tokio::task::spawn_blocking(move || {
                git_repo::prepare_worktree(&repo, &worktree, false)
            })
            .await??;
        }

        state
            .update_job_phase(domain, PipelinePhase::Validate)
            .await;
        let total = validate_provider(state, source, &worktree_dir).await?;

        {
            let mut jobs = state.jobs.write().await;
            if let Some(job) = jobs.get_mut(domain) {
                job.documents_total = total;
            }
        }

        state.update_job_phase(domain, PipelinePhase::Report).await;
        generate_report(state, source).await?;

        Ok(())
    }
    .await;
    cleanup_worktree(&worktree_dir).await;
    result
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

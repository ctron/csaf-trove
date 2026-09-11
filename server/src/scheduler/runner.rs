use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use chrono::Utc;

use crate::{
    AppState,
    models::{
        source::{Source, sanitize_domain},
        state::{JobPhase, JobStatus},
    },
    storage::git_repo,
};

/// Runs the full pipeline (sync, validate, report) for a provider with job status tracking.
pub async fn run_provider(state: &Arc<AppState>, source: &Source) -> Result<()> {
    let domain = &source.domain;
    tracing::info!("Starting pipeline for {domain}");

    let job = JobStatus {
        status: JobPhase::Running,
        started_at: Utc::now(),
        completed_at: None,
        phase: Some("sync".into()),
        documents_synced: 0,
        documents_validated: 0,
        documents_total: 0,
        error: None,
    };
    state.update_job(domain, job).await;

    let result = run_pipeline(state, source).await;

    match &result {
        Ok(()) => {
            let mut job = state.get_job(domain).await.unwrap_or_else(|| JobStatus {
                status: JobPhase::Completed,
                started_at: Utc::now(),
                completed_at: Some(Utc::now()),
                phase: None,
                documents_synced: 0,
                documents_validated: 0,
                documents_total: 0,
                error: None,
            });
            job.status = JobPhase::Completed;
            job.completed_at = Some(Utc::now());
            job.phase = None;
            state.update_job(domain, job.clone()).await;
            persist_job_counts(state, domain, &job).await;
            tracing::info!("Pipeline for {domain} completed");
        }
        Err(e) => {
            let mut job = state.get_job(domain).await.unwrap_or_else(|| JobStatus {
                status: JobPhase::Failed,
                started_at: Utc::now(),
                completed_at: Some(Utc::now()),
                phase: None,
                documents_synced: 0,
                documents_validated: 0,
                documents_total: 0,
                error: None,
            });
            job.status = JobPhase::Failed;
            job.completed_at = Some(Utc::now());
            job.error = Some(format!("{e:#}"));
            state.update_job(domain, job).await;
            tracing::error!("Pipeline for {domain} failed: {e:#}");
        }
    }

    result
}

/// Saves the final document counts from a completed job into the persisted sync state.
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
}

async fn run_pipeline(state: &Arc<AppState>, source: &Source) -> Result<()> {
    let domain = &source.domain;
    let repo_path = state.storage.repo_path(domain);
    let worktree_dir = state.work_dir().join(sanitize_domain(domain));

    git_repo::init_bare(&repo_path)?;

    setup_worktree(&repo_path, &worktree_dir)?;

    let sync_result = crate::pipeline::sync::sync_provider(state, source, &worktree_dir).await;

    if let Err(e) = &sync_result {
        tracing::error!("Sync failed for {domain}: {e:#}");
        cleanup_worktree(&worktree_dir);
        return sync_result;
    }

    git_repo::commit_all(
        &repo_path,
        &worktree_dir,
        &format!("sync: {}", Utc::now().format("%Y-%m-%dT%H:%MZ")),
    )?;

    {
        let mut jobs = state.jobs.write().await;
        if let Some(job) = jobs.get_mut(domain) {
            job.documents_total = job.documents_synced;
        }
    }

    state.update_job_phase(domain, "validate").await;
    crate::pipeline::validate::validate_provider(state, source, &worktree_dir).await?;

    state.update_job_phase(domain, "report").await;
    crate::pipeline::report::generate_report(state, source).await?;

    cleanup_worktree(&worktree_dir);
    Ok(())
}

fn setup_worktree(repo_path: &std::path::Path, worktree_dir: &PathBuf) -> Result<()> {
    if worktree_dir.exists() {
        std::fs::remove_dir_all(worktree_dir)?;
    }
    std::fs::create_dir_all(worktree_dir)?;

    let repo_str = repo_path.to_str().context("non-UTF8 repo path")?;
    let bare = git2::Repository::open_bare(repo_path)?;
    if bare.head().is_ok() {
        git2::Repository::clone(repo_str, worktree_dir)
            .context("Failed to clone bare repo to worktree")?;
    } else {
        let repo = git2::Repository::init(worktree_dir).context("Failed to init worktree")?;
        repo.remote("origin", repo_str)
            .context("Failed to add origin remote to worktree")?;
    }

    Ok(())
}

fn cleanup_worktree(worktree_dir: &PathBuf) {
    if worktree_dir.exists()
        && let Err(e) = std::fs::remove_dir_all(worktree_dir)
    {
        tracing::warn!(
            "Failed to clean up worktree {}: {e}",
            worktree_dir.display()
        );
    }
}

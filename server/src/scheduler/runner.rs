use crate::{
    AppState,
    models::{
        source::{Source, sanitize_domain},
        state::{JobPhase, JobStatus},
    },
    pipeline::store::DIR_METADATA,
    storage::{ProviderInfo, git_repo},
};
use anyhow::{Context, Result};
use csaf_walker::model::metadata::{ProviderMetadata, Role};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use time::OffsetDateTime;

/// Runs the full pipeline (sync, validate, report) for a provider with job status tracking.
///
/// Acquires a per-provider lock so concurrent runs for the same domain are skipped.
pub async fn run_provider(state: &Arc<AppState>, source: &Source) -> Result<()> {
    let domain = &source.domain;

    let lock = {
        let mut locks = state.pipeline_locks.write().await;
        locks.entry(domain.to_string()).or_default().clone()
    };
    let Ok(_guard) = lock.try_lock() else {
        tracing::warn!("Pipeline for {domain} already running, skipping");
        return Ok(());
    };

    tracing::info!("Starting pipeline for {domain}");

    let last_completed = state.get_job(domain).await.and_then(|j| j.completed_at);
    let now = OffsetDateTime::now_utc();

    let job = JobStatus {
        status: JobPhase::Running,
        started_at: now,
        completed_at: None,
        phase: Some("sync".into()),
        documents_synced: 0,
        documents_validated: 0,
        documents_total: 0,
        error: None,
        last_completed_at: last_completed,
        phase_started_at: Some(now),
    };
    state.update_job(domain, job).await;

    let result = run_pipeline(state, source).await;

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
                error: None,
                last_completed_at: None,
                phase_started_at: None,
            });
            job.status = JobPhase::Completed;
            job.completed_at = Some(OffsetDateTime::now_utc());
            job.phase = None;
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
                error: None,
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

async fn run_pipeline(state: &Arc<AppState>, source: &Source) -> Result<()> {
    let domain = &source.domain;
    let repo_path = state.storage.repo_path(domain);
    let worktree_dir = state.work_dir().join(sanitize_domain(domain));

    {
        let repo = repo_path.clone();
        tokio::task::spawn_blocking(move || git_repo::init_bare(&repo)).await??;
    }

    let sync_state = state.storage.load_sync_state(domain).await?;
    let db_count = state.storage.document_count(domain).await?;
    let incremental = sync_state.since_token.is_some() && db_count > 0;

    if incremental {
        tracing::info!(
            "{domain}: incremental mode — skipping worktree checkout ({db_count} docs in DB)"
        );
    }

    {
        let repo = repo_path.clone();
        let worktree = worktree_dir.clone();
        tokio::task::spawn_blocking(move || setup_worktree(&repo, &worktree, incremental))
            .await??;
    }

    let retrieval_errors =
        match crate::pipeline::sync::sync_provider(state, source, &worktree_dir).await {
            Ok(errors) => errors,
            Err(e) => {
                tracing::error!("Sync failed for {domain}: {e:#}");
                cleanup_worktree(&worktree_dir).await;
                return Err(e);
            }
        };

    state.update_job_phase(domain, "commit").await;
    {
        let repo = repo_path.clone();
        let worktree = worktree_dir.clone();
        let now = OffsetDateTime::now_utc();
        let msg = format!(
            "sync: {:04}-{:02}-{:02}T{:02}:{:02}Z",
            now.year(),
            now.month() as u8,
            now.day(),
            now.hour(),
            now.minute(),
        );
        tokio::task::spawn_blocking(move || git_repo::commit_all(&repo, &worktree, &msg)).await??;
    }

    persist_provider_metadata(state, domain, &worktree_dir).await;

    state.update_job_phase(domain, "validate").await;
    let total = crate::pipeline::validate::validate_provider(state, source, &worktree_dir).await?;

    {
        let mut jobs = state.jobs.write().await;
        if let Some(job) = jobs.get_mut(domain) {
            job.documents_total = total;
        }
    }

    if !retrieval_errors.is_empty() {
        let pairs: Vec<(String, String)> = retrieval_errors
            .into_iter()
            .map(|e| (e.url, e.error))
            .collect();
        state.storage.save_retrieval_errors(domain, &pairs).await?;
    }

    state.update_job_phase(domain, "report").await;
    crate::pipeline::report::generate_report(state, source).await?;

    cleanup_worktree(&worktree_dir).await;
    Ok(())
}

/// Sets up a worktree directory for the sync pipeline.
///
/// When `incremental` is false (full sync), clones the bare repo with a full checkout
/// so the worktree contains all documents. When `incremental` is true, fetches the repo
/// data and populates the git index from the HEAD tree without extracting files — only
/// documents downloaded during sync will appear in the working directory.
///
/// If the bare repo's HEAD doesn't resolve (e.g. branch name mismatch),
/// falls back to the first available branch.
fn setup_worktree(
    repo_path: &std::path::Path,
    worktree_dir: &PathBuf,
    incremental: bool,
) -> Result<()> {
    if worktree_dir.exists() {
        std::fs::remove_dir_all(worktree_dir)?;
    }
    std::fs::create_dir_all(worktree_dir)?;

    let repo_str = repo_path.to_str().context("non-UTF8 repo path")?;
    let bare = git2::Repository::open_bare(repo_path)?;

    let head_ok = bare.head().is_ok();
    if !head_ok && let Ok(branches) = bare.branches(Some(git2::BranchType::Local)) {
        for branch in branches.flatten() {
            if let Some(name) = branch.0.name().ok().flatten() {
                let target_ref = format!("refs/heads/{name}");
                if bare.set_head(&target_ref).is_ok() {
                    tracing::info!("Fixed bare repo HEAD → {target_ref}");
                    break;
                }
            }
        }
    }

    if bare.head().is_ok() {
        if incremental {
            setup_worktree_incremental(repo_path, repo_str, worktree_dir, &bare)?;
        } else {
            git2::Repository::clone(repo_str, worktree_dir)
                .context("Failed to clone bare repo to worktree")?;
        }
    } else {
        let repo = git2::Repository::init(worktree_dir).context("Failed to init worktree")?;
        repo.remote("origin", repo_str)
            .context("Failed to add origin remote to worktree")?;
    }

    Ok(())
}

/// Fetches from the bare repo and populates the index without checking out files.
///
/// This leaves the working directory empty so that only documents downloaded during
/// sync appear on disk. The git index still contains all entries from the HEAD tree,
/// so `commit_all` produces a complete tree when it adds the new working-directory files.
fn setup_worktree_incremental(
    repo_path: &std::path::Path,
    repo_str: &str,
    worktree_dir: &PathBuf,
    bare: &git2::Repository,
) -> Result<()> {
    let repo = git2::Repository::init(worktree_dir).context("Failed to init worktree")?;
    let mut remote = repo
        .remote("origin", repo_str)
        .context("Failed to add origin remote")?;
    remote
        .fetch(&["refs/heads/*:refs/remotes/origin/*"], None, None)
        .context("Failed to fetch from bare repo")?;

    let head_ref = bare.head().context("bare repo has no HEAD")?;
    let branch_name = head_ref.shorthand().unwrap_or("main");

    let remote_ref = format!("refs/remotes/origin/{branch_name}");
    let reference = repo
        .find_reference(&remote_ref)
        .with_context(|| format!("remote ref {remote_ref} not found after fetch"))?;
    let commit = reference
        .peel_to_commit()
        .context("remote ref does not point to a commit")?;

    let mut index = repo.index().context("failed to open worktree index")?;
    index
        .read_tree(&commit.tree()?)
        .context("failed to populate index from HEAD tree")?;
    index.write().context("failed to write index")?;

    repo.branch(branch_name, &commit, false)
        .context("failed to create local branch")?;
    repo.set_head(&format!("refs/heads/{branch_name}"))
        .context("failed to set HEAD")?;

    let file_count = index.len();
    tracing::debug!(
        "Incremental worktree: index has {file_count} entries, working directory is empty ({})",
        repo_path.display()
    );

    Ok(())
}

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

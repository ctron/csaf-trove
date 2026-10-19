//! Server startup: the HTTP server binds right away, while the slow initialization runs in the
//! background and reports its progress through `GET /api/startup`.
//!
//! Until startup has finished, data endpoints answer with `503 Service Unavailable`, while the
//! dashboard assets are served so the UI can show the startup progress.
#[cfg(test)]
mod test;

use crate::{
    AppState, config_sync, hooks, load_sources_from_dir,
    models::state::{JobPhase, JobStatus},
    scheduler,
};
use actix_web::{
    Error, HttpResponse,
    body::{BoxBody, MessageBody},
    dev::{ServiceRequest, ServiceResponse},
    http::header::RETRY_AFTER,
    middleware::Next,
    web,
};
use csaf_trove_common::startup::StartupStatus;
use std::sync::Arc;
use tokio::{sync::watch, task::spawn_blocking};

/// Path prefixes that are unavailable until startup has finished.
const GATED_PREFIXES: &[&str] = &["/api/", "/mcp", "/.well-known/"];

/// Path of the startup status endpoint, which is always available.
const STATUS_PATH: &str = "/api/startup";

/// Seconds clients are asked to wait before retrying a request during startup.
const RETRY_AFTER_SECS: u64 = 5;

/// Tracks the progress of server startup.
pub struct Startup {
    /// Current startup status.
    status: watch::Sender<StartupStatus>,
}

impl Default for Startup {
    fn default() -> Self {
        Self {
            status: watch::channel(StartupStatus::default()).0,
        }
    }
}

impl Startup {
    /// Creates a tracker for a server that has already finished starting up.
    #[cfg(test)]
    pub fn finished() -> Self {
        let startup = Self::default();
        startup.finish();
        startup
    }

    /// Starts a new task, marking the previous one as completed.
    pub fn begin(&self, task: &str) {
        tracing::info!("Startup: {task}");
        self.status.send_modify(|status| {
            status.completed.extend(status.task.take());
            status.task = Some(task.to_string());
            status.current = 0;
            status.total = 0;
        });
    }

    /// Updates the progress within the current task.
    pub fn progress(&self, current: u64, total: u64) {
        self.status.send_modify(|status| {
            status.current = current;
            status.total = total;
        });
    }

    /// Marks startup as finished.
    pub fn finish(&self) {
        self.status.send_modify(|status| {
            status.completed.extend(status.task.take());
            status.current = 0;
            status.total = 0;
            status.ready = true;
        });
    }

    /// Returns `true` once startup has finished.
    pub fn is_ready(&self) -> bool {
        self.status.borrow().ready
    }

    /// Returns the current startup status.
    pub fn snapshot(&self) -> StartupStatus {
        self.status.borrow().clone()
    }
}

/// Runs the startup tasks, then starts the background services.
pub async fn run(state: Arc<AppState>) {
    let startup = &state.startup;

    if let Some(github) = &state.config.github {
        startup.begin("Synchronizing configuration");
        let sync_dir = state.data_dir.clone();
        let sync_url = github.repo.clone();
        match spawn_blocking(move || config_sync::sync(&sync_dir, &sync_url)).await {
            Ok(Ok(())) => tracing::info!("Initial config sync complete"),
            Ok(Err(e)) => tracing::warn!("Initial config sync failed: {e:#}"),
            Err(e) => tracing::warn!("Config sync task failed: {e:#}"),
        }
    }

    startup.begin("Loading provider sources");
    load_sources(&state).await;

    startup.begin("Restoring job statuses");
    restore_jobs(&state).await;

    startup.begin("Loading sync history");
    load_sync_points(&state).await;

    startup.finish();
    tracing::info!("Startup complete");

    hooks::spawn(&state);
    // Warm the overview cache so the first page load doesn't pay for the storage walk.
    let overview_state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = overview_state.storage.overview_snapshot().await {
            tracing::warn!("Failed to compute overview snapshot: {e:#}");
        }
    });

    scheduler::run(state).await;
}

/// Loads provider sources from the sources directory, plus the inline sources of the config.
async fn load_sources(state: &AppState) {
    let mut sources = load_sources_from_dir(&state.data_dir.join("sources"))
        .await
        .unwrap_or_default();

    for s in &state.config.source {
        sources.insert(s.domain.clone(), s.clone());
    }
    if !state.config.source.is_empty() {
        tracing::info!(
            "Added {} inline source(s) from config",
            state.config.source.len()
        );
    }

    tracing::info!("Loaded {} sources", sources.len());
    *state.sources.write().await = sources;
}

/// Restores the status of completed jobs from the persisted sync states.
async fn restore_jobs(state: &AppState) {
    let Ok(sync_states) = state.storage.list_sync_states().await else {
        return;
    };

    let total = sync_states.len() as u64;
    let mut restored = Vec::new();
    for (i, ss) in sync_states.into_iter().enumerate() {
        state.startup.progress(i as u64, total);
        if let Some(last_sync) = ss.last_sync {
            let duration = state
                .storage
                .last_sync_duration(&ss.domain)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!("Failed to load last sync duration for {}: {e}", ss.domain);
                    None
                })
                .unwrap_or_default();
            restored.push((ss, last_sync, duration));
        }
    }
    state.startup.progress(total, total);

    let mut jobs = state.jobs.write().await;
    for (ss, last_sync, duration) in restored {
        jobs.insert(
            ss.domain.clone(),
            JobStatus {
                status: JobPhase::Completed,
                started_at: last_sync - duration,
                completed_at: Some(last_sync),
                phase: None,
                documents_synced: ss.documents_synced,
                documents_validated: ss.documents_validated,
                documents_missing: ss.documents_missing,
                documents_total: ss.documents_total,
                distributions_total: 0,
                distribution_index: 0,
                distribution_documents_current: 0,
                distribution_documents_total: 0,
                phase_current: 0,
                phase_total: 0,
                error: None,
                completed_phases: vec![],
                last_completed_at: Some(last_sync),
                phase_started_at: None,
            },
        );
    }
    tracing::info!("Restored {} job statuses from persisted state", jobs.len());
}

/// Loads the recent sync points of all providers for the sparklines.
async fn load_sync_points(state: &AppState) {
    let domains: Vec<String> = state.sources.read().await.keys().cloned().collect();
    let total = domains.len() as u64;
    let mut loaded = Vec::new();
    for (i, domain) in domains.into_iter().enumerate() {
        state.startup.progress(i as u64, total);
        if let Ok(Some(p)) = state.storage.recent_sync_points(&domain, 30).await {
            loaded.push((domain, p));
        }
    }
    state.startup.progress(total, total);
    state.recent_sync_points.write().await.extend(loaded);
}

/// Returns the current startup status.
pub async fn status(state: web::Data<AppState>) -> HttpResponse {
    HttpResponse::Ok().json(state.startup.snapshot())
}

/// Returns `true` if the path is unavailable until startup has finished.
fn is_gated(path: &str) -> bool {
    path != STATUS_PATH && GATED_PREFIXES.iter().any(|p| path.starts_with(p))
}

/// Rejects requests to data endpoints with `503 Service Unavailable` until startup has finished.
pub async fn middleware(
    req: ServiceRequest,
    next: Next<impl MessageBody + 'static>,
) -> Result<ServiceResponse<BoxBody>, Error> {
    let starting = req
        .app_data::<web::Data<AppState>>()
        .is_some_and(|state| !state.startup.is_ready());

    if starting && is_gated(req.path()) {
        let response = HttpResponse::ServiceUnavailable()
            .insert_header((RETRY_AFTER, RETRY_AFTER_SECS))
            .body("Server is starting up, please retry later");
        return Ok(req.into_response(response));
    }

    Ok(next.call(req).await?.map_into_boxed_body())
}

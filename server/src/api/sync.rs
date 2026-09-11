use std::collections::HashMap;

use actix_web::{HttpRequest, HttpResponse, web};
use chrono::Utc;
use serde::Serialize;

use super::{auth::verify_bearer_token, error::ApiError};
use crate::{
    AppState,
    models::state::{JobPhase, JobStatus},
};

/// API response wrapper that adds computed duration to a job status.
#[derive(Serialize)]
struct SyncStatusEntry {
    /// The underlying job status fields.
    #[serde(flatten)]
    job: JobStatus,
    /// Elapsed seconds (running) or total seconds (completed/failed).
    duration_seconds: Option<f64>,
}

/// Returns the current job status for all providers with computed durations.
pub async fn status(state: web::Data<AppState>) -> Result<HttpResponse, ApiError> {
    let jobs = state.jobs.read().await;
    let now = Utc::now();

    let entries: HashMap<String, SyncStatusEntry> = jobs
        .iter()
        .map(|(domain, job)| {
            let duration_seconds = match job.status {
                JobPhase::Running | JobPhase::Pending => {
                    Some((now - job.started_at).num_milliseconds() as f64 / 1000.0)
                }
                JobPhase::Completed | JobPhase::Failed => job
                    .completed_at
                    .map(|end| (end - job.started_at).num_milliseconds() as f64 / 1000.0),
            };
            (
                domain.clone(),
                SyncStatusEntry {
                    job: job.clone(),
                    duration_seconds,
                },
            )
        })
        .collect();

    Ok(HttpResponse::Ok().json(&entries))
}

/// Triggers a manual sync for a single provider (requires Bearer token).
pub async fn trigger(
    req: HttpRequest,
    state: web::Data<AppState>,
    domain: web::Path<String>,
) -> Result<HttpResponse, ApiError> {
    let token = state.api_token.as_ref().ok_or(ApiError::Forbidden)?;

    if !verify_bearer_token(&req, token) {
        return Err(ApiError::Unauthorized);
    }

    let domain = domain.into_inner();

    let source = state
        .sources
        .read()
        .await
        .get(&domain)
        .cloned()
        .ok_or(ApiError::NotFound)?;

    let state = state.into_inner();
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        handle.block_on(async move {
            if let Err(e) = crate::scheduler::runner::run_provider(&state, &source).await {
                tracing::error!("Manual sync for {} failed: {e}", source.domain);
            }
        });
    });

    Ok(HttpResponse::Accepted().json(serde_json::json!({ "status": "started" })))
}

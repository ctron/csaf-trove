use std::{collections::HashMap, sync::Arc};

use actix_web::{HttpRequest, HttpResponse, web};
use csaf_trove_common::SyncPoint;
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::{auth::verify_bearer_token, error::ApiError};
use crate::{
    AppState,
    models::state::{JobPhase, JobStatus},
};

/// API response wrapper that adds computed fields to a job status.
#[derive(Serialize)]
struct SyncStatusEntry {
    /// The underlying job status fields.
    #[serde(flatten)]
    job: JobStatus,
    /// Elapsed seconds (running) or total seconds (completed/failed).
    duration_seconds: Option<f64>,
    /// Pre-formatted ETA string (e.g. "~5m 30s"), only while running.
    #[serde(skip_serializing_if = "Option::is_none")]
    eta: Option<String>,
    /// When the provider last completed a sync (ISO 8601).
    #[serde(skip_serializing_if = "Option::is_none")]
    last_run: Option<String>,
    /// Recent sync points in chronological order for sparkline rendering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    recent_sync_points: Vec<SyncPoint>,
}

/// Builds status entries with computed durations for all providers.
async fn build_status_entries(state: &AppState) -> HashMap<String, SyncStatusEntry> {
    let jobs = state.jobs.read().await;
    let sparklines = state.recent_sync_points.read().await;
    let now = OffsetDateTime::now_utc();

    jobs.iter()
        .map(|(domain, job)| {
            let duration_seconds = match job.status {
                JobPhase::Running | JobPhase::Pending => {
                    Some((now - job.started_at).as_seconds_f64())
                }
                JobPhase::Completed | JobPhase::Failed => job
                    .completed_at
                    .map(|end| (end - job.started_at).as_seconds_f64()),
            };

            let eta = if job.status == JobPhase::Running {
                compute_eta(job, now)
            } else {
                None
            };

            let last_run = match job.status {
                JobPhase::Completed | JobPhase::Failed => {
                    job.completed_at.and_then(|t| t.format(&Rfc3339).ok())
                }
                _ => job.last_completed_at.and_then(|t| t.format(&Rfc3339).ok()),
            };

            let recent_sync_points = sparklines.get(domain).cloned().unwrap_or_default();

            (
                domain.clone(),
                SyncStatusEntry {
                    job: job.clone(),
                    duration_seconds,
                    eta,
                    last_run,
                    recent_sync_points,
                },
            )
        })
        .collect()
}

/// Computes a human-readable ETA for a running job based on the current phase progress.
fn compute_eta(job: &JobStatus, now: OffsetDateTime) -> Option<String> {
    let phase_start = job.phase_started_at?;
    let elapsed = (now - phase_start).as_seconds_f64();
    if elapsed <= 0.0 {
        return None;
    }

    let current = match job.phase.as_deref() {
        Some("sync") => job.documents_synced,
        Some("validate") => job.documents_validated,
        _ => return None,
    };

    if current == 0 || job.documents_total == 0 {
        return None;
    }

    let remaining = job.documents_total.saturating_sub(current) as f64;
    let rate = current as f64 / elapsed;
    let eta_secs = (remaining / rate) as u64;

    let hours = eta_secs / 3600;
    let minutes = (eta_secs % 3600) / 60;
    let secs = eta_secs % 60;

    let formatted = if hours > 0 {
        format!("~{hours}h {minutes}m {secs}s")
    } else if minutes > 0 {
        format!("~{minutes}m {secs}s")
    } else {
        format!("~{secs}s")
    };

    Some(formatted)
}

/// Returns the current job status for all providers with computed durations.
pub async fn status(state: web::Data<AppState>) -> Result<HttpResponse, ApiError> {
    let entries = build_status_entries(&state).await;
    Ok(HttpResponse::Ok().json(&entries))
}

/// WebSocket endpoint that pushes sync status updates to connected clients.
pub async fn ws(
    req: HttpRequest,
    body: web::Payload,
    state: web::Data<AppState>,
) -> Result<HttpResponse, actix_web::Error> {
    let (response, mut session, mut msg_stream) = actix_ws::handle(&req, body)?;

    let state: Arc<AppState> = state.into_inner();
    let mut rx = state.job_notify.subscribe();

    actix_web::rt::spawn(async move {
        let send_state = |session: &mut actix_ws::Session, state: &Arc<AppState>| {
            let mut session = session.clone();
            let state = state.clone();
            async move {
                let entries = build_status_entries(&state).await;
                if let Ok(json) = serde_json::to_string(&entries) {
                    session.text(json).await.ok();
                }
            }
        };

        send_state(&mut session, &state).await;

        loop {
            tokio::select! {
                result = rx.changed() => {
                    if result.is_err() {
                        break;
                    }
                    send_state(&mut session, &state).await;
                }
                msg = msg_stream.recv() => {
                    match msg {
                        Some(Ok(actix_ws::Message::Ping(bytes))) => {
                            if session.pong(&bytes).await.is_err() {
                                break;
                            }
                        }
                        Some(Ok(actix_ws::Message::Close(_))) | None => {
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        session.close(None).await.ok();
    });

    Ok(response)
}

/// Query parameters for the sync trigger endpoint.
#[derive(Debug, Deserialize)]
pub struct TriggerQuery {
    /// When `true`, clears the since-token to force a full re-sync.
    #[serde(default)]
    pub full: bool,
}

/// Triggers a manual sync for a single provider (requires Bearer token).
pub async fn trigger(
    req: HttpRequest,
    state: web::Data<AppState>,
    domain: web::Path<String>,
    query: web::Query<TriggerQuery>,
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

    if query.full {
        let mut sync_state = state.storage.load_sync_state(&domain).await?;
        sync_state.since_token = None;
        state.storage.save_sync_state(&sync_state).await?;
        tracing::info!("{domain}: cleared since_token for full re-sync");
    }

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

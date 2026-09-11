use actix_web::{HttpRequest, HttpResponse, web};

use super::{auth::verify_bearer_token, error::ApiError};
use crate::AppState;

/// Returns the current job status for all providers.
pub async fn status(state: web::Data<AppState>) -> Result<HttpResponse, ApiError> {
    let jobs = state.jobs.read().await;
    Ok(HttpResponse::Ok().json(&*jobs))
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

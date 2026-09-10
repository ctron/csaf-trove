use actix_web::{HttpRequest, HttpResponse, web};

use crate::AppState;

/// Returns the current job status for all providers.
pub async fn status(state: web::Data<AppState>) -> HttpResponse {
    let jobs = state.jobs.read().await;
    HttpResponse::Ok().json(&*jobs)
}

/// Triggers a manual sync for a single provider (requires Bearer token).
pub async fn trigger(
    req: HttpRequest,
    state: web::Data<AppState>,
    domain: web::Path<String>,
) -> HttpResponse {
    let Some(ref token) = state.api_token else {
        return HttpResponse::Forbidden().finish();
    };

    if !verify_bearer_token(&req, token) {
        return HttpResponse::Unauthorized().finish();
    }

    let domain = domain.into_inner();

    let source = state.sources.read().await.get(&domain).cloned();

    if let Some(source) = source {
        let state = state.into_inner();
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            handle.block_on(async move {
                if let Err(e) = crate::scheduler::runner::run_provider(&state, &source).await {
                    tracing::error!("Manual sync for {} failed: {e}", source.domain);
                }
            });
        });
        HttpResponse::Accepted().json(serde_json::json!({ "status": "started" }))
    } else {
        HttpResponse::NotFound().finish()
    }
}

fn verify_bearer_token(req: &HttpRequest, expected: &str) -> bool {
    req.headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|token| token == expected)
}

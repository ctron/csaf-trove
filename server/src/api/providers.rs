use actix_web::{HttpResponse, web};

use crate::AppState;

/// Returns all provider summaries as JSON.
pub async fn list(state: web::Data<AppState>) -> HttpResponse {
    match state.storage.list_summaries().await {
        Ok(providers) => HttpResponse::Ok().json(providers),
        Err(e) => {
            tracing::error!("Failed to list providers: {e}");
            HttpResponse::InternalServerError().finish()
        }
    }
}

/// Returns the detail view (summary + metrics) for a single provider.
pub async fn detail(state: web::Data<AppState>, domain: web::Path<String>) -> HttpResponse {
    let domain = domain.into_inner();
    match state.storage.provider_detail(&domain).await {
        Ok(Some(detail)) => HttpResponse::Ok().json(detail),
        Ok(None) => HttpResponse::NotFound().finish(),
        Err(e) => {
            tracing::error!("Failed to get provider {domain}: {e}");
            HttpResponse::InternalServerError().finish()
        }
    }
}

/// Returns the git commit history for a provider's document repository.
pub async fn history(state: web::Data<AppState>, domain: web::Path<String>) -> HttpResponse {
    let domain = domain.into_inner();
    match state.storage.provider_history(&domain).await {
        Ok(Some(history)) => HttpResponse::Ok().json(history),
        Ok(None) => HttpResponse::NotFound().finish(),
        Err(e) => {
            tracing::error!("Failed to get history for {domain}: {e}");
            HttpResponse::InternalServerError().finish()
        }
    }
}

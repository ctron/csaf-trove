use actix_web::{HttpResponse, web};
use serde::Deserialize;

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

/// Query parameters for the document listing endpoint.
#[derive(Debug, Deserialize)]
pub struct DocumentsQuery {
    /// Zero-based offset for pagination.
    pub offset: Option<u64>,
    /// Maximum number of results (default 50, max 200).
    pub limit: Option<u64>,
    /// Filter by status: `passing`, `failing`, or omit for all.
    pub status: Option<String>,
}

/// Returns paginated per-document validation results for a provider.
pub async fn documents(
    state: web::Data<AppState>,
    domain: web::Path<String>,
    query: web::Query<DocumentsQuery>,
) -> HttpResponse {
    let domain = domain.into_inner();
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(50).min(200);
    let status = query.status.as_deref();

    match state
        .storage
        .load_documents_paginated(&domain, offset, limit, status)
    {
        Ok(Some(page)) => HttpResponse::Ok().json(page),
        Ok(None) => HttpResponse::NotFound().finish(),
        Err(e) => {
            tracing::error!("Failed to load documents for {domain}: {e}");
            HttpResponse::InternalServerError().finish()
        }
    }
}

/// Returns a single document's validation results by tracking ID.
pub async fn document_detail(
    state: web::Data<AppState>,
    path: web::Path<(String, String)>,
) -> HttpResponse {
    let (domain, tracking_id) = path.into_inner();

    match state.storage.load_document(&domain, &tracking_id) {
        Ok(Some(doc)) => HttpResponse::Ok().json(doc),
        Ok(None) => HttpResponse::NotFound().finish(),
        Err(e) => {
            tracing::error!("Failed to load document {tracking_id} for {domain}: {e}");
            HttpResponse::InternalServerError().finish()
        }
    }
}

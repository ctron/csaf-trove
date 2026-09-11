use actix_web::{HttpRequest, HttpResponse, web};
use serde::Deserialize;

use super::{
    auth::verify_bearer_token,
    error::{ApiError, OptionExt},
};
use crate::{AppState, models::state::JobPhase};

/// Returns all provider summaries as JSON.
pub async fn list(state: web::Data<AppState>) -> Result<HttpResponse, ApiError> {
    let providers = state.storage.list_summaries().await?;
    Ok(HttpResponse::Ok().json(providers))
}

/// Returns the detail view (summary + metrics) for a single provider.
pub async fn detail(
    state: web::Data<AppState>,
    domain: web::Path<String>,
) -> Result<HttpResponse, ApiError> {
    let domain = domain.into_inner();
    let detail = state
        .storage
        .provider_detail(&domain)
        .await?
        .or_not_found()?;
    Ok(HttpResponse::Ok().json(detail))
}

/// Returns the git commit history for a provider's document repository.
pub async fn history(
    state: web::Data<AppState>,
    domain: web::Path<String>,
) -> Result<HttpResponse, ApiError> {
    let domain = domain.into_inner();
    let history = state
        .storage
        .provider_history(&domain)
        .await?
        .or_not_found()?;
    Ok(HttpResponse::Ok().json(history))
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
) -> Result<HttpResponse, ApiError> {
    let domain = domain.into_inner();
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(50).min(200);
    let status = query.status.as_deref();

    let page = state
        .storage
        .load_documents_paginated(&domain, offset, limit, status)?
        .or_not_found()?;
    Ok(HttpResponse::Ok().json(page))
}

/// Deletes all stored data for a provider (requires Bearer token).
pub async fn delete(
    req: HttpRequest,
    state: web::Data<AppState>,
    domain: web::Path<String>,
) -> Result<HttpResponse, ApiError> {
    let token = state.api_token.as_ref().ok_or(ApiError::Forbidden)?;

    if !verify_bearer_token(&req, token) {
        return Err(ApiError::Unauthorized);
    }

    let domain = domain.into_inner();

    if let Some(job) = state.get_job(&domain).await
        && job.status == JobPhase::Running
    {
        return Err(ApiError::Conflict(
            "sync is running for this provider".into(),
        ));
    }

    let in_sources = state.sources.read().await.contains_key(&domain);
    let has_data = state.storage.has_provider_data(&domain);

    if !in_sources && !has_data {
        return Err(ApiError::NotFound);
    }

    state.storage.delete_provider(&domain).await?;

    state.sources.write().await.remove(&domain);
    state.jobs.write().await.remove(&domain);

    tracing::info!("Deleted provider data for {domain}");
    Ok(HttpResponse::NoContent().finish())
}

/// Returns a single document's validation results by tracking ID.
pub async fn document_detail(
    state: web::Data<AppState>,
    path: web::Path<(String, String)>,
) -> Result<HttpResponse, ApiError> {
    let (domain, tracking_id) = path.into_inner();
    let doc = state
        .storage
        .load_document(&domain, &tracking_id)?
        .or_not_found()?;
    Ok(HttpResponse::Ok().json(doc))
}

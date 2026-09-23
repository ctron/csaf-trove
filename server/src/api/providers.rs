use actix_web::{HttpRequest, HttpResponse, web};
use serde::Deserialize;

use super::{
    auth::verify_bearer_token,
    error::{ApiError, OptionExt},
};
use crate::{
    AppState,
    models::{
        result::{ProfileResults, ProviderDetail, ProviderSummary},
        state::JobPhase,
    },
};

/// Returns all provider summaries as JSON, including placeholders for sources awaiting first sync.
pub async fn list(state: web::Data<AppState>) -> Result<HttpResponse, ApiError> {
    let mut providers = state.storage.list_summaries().await?;

    let sources = state.sources.read().await;
    let known: std::collections::HashSet<String> =
        providers.iter().map(|p| p.provider.clone()).collect();

    for (domain, source) in sources.iter() {
        if source.enabled && !known.contains(domain) {
            providers.push(ProviderSummary {
                provider: domain.clone(),
                publisher_name: None,
                validated_at: time::OffsetDateTime::now_utc(),
                document_count: 0,
                profiles: ProfileResults {
                    basic: None,
                    extended: None,
                    full: None,
                },
                signatures: None,
                top_failing_tests: vec![],
                retrieval_errors: 0,
                note: source.note.clone(),
            });
        }
    }

    for p in &mut providers {
        if p.note.is_none() {
            p.note = sources.get(&p.provider).and_then(|s| s.note.clone());
        }
    }

    providers.sort_by(|a, b| a.provider.cmp(&b.provider));
    Ok(HttpResponse::Ok().json(providers))
}

/// Returns the detail view (summary + metrics) for a single provider.
pub async fn detail(
    state: web::Data<AppState>,
    domain: web::Path<String>,
) -> Result<HttpResponse, ApiError> {
    let domain = domain.into_inner();
    let sources = state.sources.read().await;
    let source = sources.get(&domain);
    let skip_directories = source
        .map(|s| s.skip_directories.clone())
        .unwrap_or_default();
    let note = source.and_then(|s| s.note.clone());
    let is_known_source = source.is_some();
    drop(sources);
    let detail = state
        .storage
        .provider_detail(&domain, &skip_directories)
        .await?;
    match detail {
        Some(mut d) => {
            d.note = note;
            Ok(HttpResponse::Ok().json(d))
        }
        None if is_known_source => {
            let stub = ProviderDetail {
                summary: ProviderSummary {
                    provider: domain,
                    publisher_name: None,
                    validated_at: time::OffsetDateTime::now_utc(),
                    document_count: 0,
                    profiles: ProfileResults {
                        basic: None,
                        extended: None,
                        full: None,
                    },
                    signatures: None,
                    top_failing_tests: vec![],
                    retrieval_errors: 0,
                    note: None,
                },
                metrics: None,
                history: vec![],
                distributions: vec![],
                note,
            };
            Ok(HttpResponse::Ok().json(stub))
        }
        None => Err(ApiError::NotFound),
    }
}

/// Query parameters for the sync history endpoint.
#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    /// Zero-based offset for pagination.
    pub offset: Option<u64>,
    /// Maximum number of results (default 50, max 200).
    pub limit: Option<u64>,
}

/// Returns paginated sync run history for a provider.
pub async fn history(
    state: web::Data<AppState>,
    domain: web::Path<String>,
    query: web::Query<HistoryQuery>,
) -> Result<HttpResponse, ApiError> {
    let domain = domain.into_inner();
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(50).min(200);

    let page = state
        .storage
        .provider_history_paginated(&domain, offset, limit)
        .await?
        .or_not_found()?;
    Ok(HttpResponse::Ok().json(page))
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
        .load_documents_paginated(&domain, offset, limit, status)
        .await?
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
        .load_document(&domain, &tracking_id)
        .await?
        .or_not_found()?;
    Ok(HttpResponse::Ok().json(doc))
}

/// Returns the version history for a specific document from git.
pub async fn document_versions(
    state: web::Data<AppState>,
    path: web::Path<(String, String)>,
) -> Result<HttpResponse, ApiError> {
    let (domain, tracking_id) = path.into_inner();
    let versions = state
        .storage
        .document_versions(&domain, &tracking_id)
        .await?
        .or_not_found()?;
    Ok(HttpResponse::Ok().json(versions))
}

/// Returns metadata from a historical version of a document.
pub async fn document_version_detail(
    state: web::Data<AppState>,
    path: web::Path<(String, String, String)>,
) -> Result<HttpResponse, ApiError> {
    let (domain, tracking_id, commit_id) = path.into_inner();
    let doc = state
        .storage
        .read_historical_document(&domain, &tracking_id, &commit_id)
        .await?
        .or_not_found()?;
    Ok(HttpResponse::Ok().json(doc))
}

/// Returns a structured diff between a document version and its next newer version.
pub async fn document_diff(
    state: web::Data<AppState>,
    path: web::Path<(String, String, String)>,
) -> Result<HttpResponse, ApiError> {
    let (domain, tracking_id, commit_id) = path.into_inner();
    let diff = state
        .storage
        .diff_document_versions(&domain, &tracking_id, &commit_id)
        .await?
        .or_not_found()?;
    Ok(HttpResponse::Ok().json(diff))
}

use super::{error::ApiError, providers::list_summaries};
use crate::AppState;
use actix_web::{HttpResponse, web};
use csaf_trove_common::overview::Overview;

/// Returns aggregate provider counts and persistent storage usage.
pub async fn get(state: web::Data<AppState>) -> Result<HttpResponse, ApiError> {
    let providers = list_summaries(&state).await?;
    let overview = Overview {
        provider_count: providers.len() as u64,
        document_count: providers.iter().map(|p| p.document_count).sum(),
        retrieval_errors: providers.iter().map(|p| p.retrieval_errors).sum(),
        storage: state.storage.storage_usage().await?,
    };
    Ok(HttpResponse::Ok().json(overview))
}

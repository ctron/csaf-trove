use super::error::ApiError;
use crate::AppState;
use actix_web::{HttpResponse, web};

/// Serves the generated `aggregator.json`.
pub async fn get(state: web::Data<AppState>) -> Result<HttpResponse, ApiError> {
    let path = state.aggregator_dir().join("aggregator.json");

    let data = tokio::fs::read(&path)
        .await
        .map_err(|_| ApiError::NotFound)?;

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .body(data))
}

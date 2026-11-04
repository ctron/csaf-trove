use super::error::ApiError;
use crate::AppState;
use actix_web::{HttpResponse, web};

/// Serves the most recently generated `aggregator.json`.
pub async fn get(state: web::Data<AppState>) -> Result<HttpResponse, ApiError> {
    let data = state
        .aggregator
        .read()
        .await
        .clone()
        .ok_or(ApiError::NotFound)?;

    Ok(HttpResponse::Ok()
        .content_type("application/json")
        .body(data))
}

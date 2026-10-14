use super::error::ApiError;
use crate::AppState;
use actix_web::{HttpResponse, web};
use serde::Deserialize;

/// Query parameters for the CVE search endpoint.
#[derive(Debug, Deserialize)]
pub struct CveQuery {
    /// Match CVE IDs starting with the given value instead of exactly.
    #[serde(default)]
    pub prefix: bool,
}

/// Returns documents across all providers that reference a CVE.
pub async fn find(
    state: web::Data<AppState>,
    cve: web::Path<String>,
    query: web::Query<CveQuery>,
) -> Result<HttpResponse, ApiError> {
    let result = state.storage.find_by_cve(&cve, query.prefix).await?;
    Ok(HttpResponse::Ok().json(&*result))
}

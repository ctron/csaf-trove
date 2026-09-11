use actix_web::{HttpRequest, HttpResponse, web};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use super::error::ApiError;
use crate::AppState;

/// Handles GitHub push webhook events by reloading source configs.
pub async fn github(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: web::Bytes,
) -> Result<HttpResponse, ApiError> {
    let secret = state.webhook_secret.as_ref().ok_or(ApiError::Forbidden)?;

    if !verify_signature(&req, &body, secret) {
        return Err(ApiError::Unauthorized);
    }

    tracing::info!("GitHub webhook received, syncing config repo");
    state.sync_and_reload_sources().await;
    Ok(HttpResponse::Ok().json(serde_json::json!({ "status": "reloaded" })))
}

fn verify_signature(req: &HttpRequest, body: &[u8], secret: &str) -> bool {
    let Some(signature) = req
        .headers()
        .get("X-Hub-Signature-256")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("sha256="))
    else {
        return false;
    };

    let Ok(signature_bytes) = hex::decode(signature) else {
        return false;
    };

    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&signature_bytes).is_ok()
}

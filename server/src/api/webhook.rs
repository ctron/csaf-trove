use actix_web::{HttpRequest, HttpResponse, web};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::AppState;

/// Handles GitHub push webhook events by reloading source configs.
pub async fn github(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: web::Bytes,
) -> HttpResponse {
    let Some(ref secret) = state.webhook_secret else {
        return HttpResponse::Forbidden().finish();
    };

    if !verify_signature(&req, &body, secret) {
        return HttpResponse::Unauthorized().finish();
    }

    tracing::info!("GitHub webhook received, triggering source config reload");
    state.reload_sources().await;
    HttpResponse::Ok().json(serde_json::json!({ "status": "reloaded" }))
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

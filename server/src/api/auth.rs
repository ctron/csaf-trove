use actix_web::HttpRequest;

/// Verifies that the request carries a valid `Authorization: Bearer <token>` header.
pub fn verify_bearer_token(req: &HttpRequest, expected: &str) -> bool {
    req.headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|token| token == expected)
}

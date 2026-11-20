use actix_web::{HttpResponse, http::StatusCode};
use serde::Serialize;

/// Structured JSON body returned for all error responses.
#[derive(Debug, Serialize)]
pub struct ErrorInformation {
    /// Human-readable error message.
    pub message: String,
    /// Machine-readable error type slug.
    #[serde(rename = "type")]
    pub r#type: String,
    /// Optional additional context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// API-layer error type mapping to HTTP status codes and JSON bodies.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// The requested resource was not found.
    #[error("resource not found")]
    NotFound,

    /// The request lacks valid authentication credentials.
    #[error("unauthorized")]
    Unauthorized,

    /// The server refuses to authorize the request.
    #[error("forbidden")]
    Forbidden,

    /// The provider's metadata disallows mirroring, so the full document is not served.
    #[error(
        "provider does not allow mirroring (mirror_on_CSAF_aggregators: false); the full document is not served"
    )]
    MirroringDisabled,

    /// The request conflicts with the current resource state.
    #[error("{0}")]
    Conflict(String),

    /// An unexpected internal error occurred.
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl actix_web::ResponseError for ApiError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden | Self::MirroringDisabled => StatusCode::FORBIDDEN,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        if let Self::Internal(e) = self {
            tracing::error!("{e:#}");
        }

        let message = match self {
            Self::Internal(_) => "internal server error".to_string(),
            other => other.to_string(),
        };

        let info = ErrorInformation {
            message,
            r#type: match self {
                Self::NotFound => "not-found",
                Self::Unauthorized => "unauthorized",
                Self::Forbidden => "forbidden",
                Self::MirroringDisabled => "mirroring-disabled",
                Self::Conflict(_) => "conflict",
                Self::Internal(_) => "internal-error",
            }
            .to_string(),
            details: None,
        };

        HttpResponse::build(self.status_code()).json(info)
    }
}

/// Extension trait for converting `Option` into `ApiError::NotFound`.
pub trait OptionExt<T> {
    /// Returns the contained value or `ApiError::NotFound`.
    fn or_not_found(self) -> Result<T, ApiError>;
}

impl<T> OptionExt<T> for Option<T> {
    fn or_not_found(self) -> Result<T, ApiError> {
        self.ok_or(ApiError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::ResponseError;

    /// A mirroring refusal maps to 403, so clients can tell it apart from a missing resource.
    #[test]
    fn mirroring_disabled_is_forbidden() {
        let err = ApiError::MirroringDisabled;
        assert_eq!(err.status_code(), StatusCode::FORBIDDEN);
        assert_eq!(err.error_response().status(), StatusCode::FORBIDDEN);
    }
}

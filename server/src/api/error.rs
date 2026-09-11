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
            Self::Forbidden => StatusCode::FORBIDDEN,
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

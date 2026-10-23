use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use outpost_core::CoreError;
use serde::Serialize;
use utoipa::ToSchema;

/// Every error the API returns has this shape.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorDetail {
    /// Stable machine code: `unauthorized`, `forbidden`, `not_found`, `conflict`,
    /// `invalid`, `bad_secret`, `locked` or `internal`.
    pub code: String,
    /// Human-readable message, safe to show in the UI.
    pub message: String,
    /// Seconds until a locked account can try again (`locked` only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<i64>,
    /// Wrong attempts left before the account locks (`bad_secret` only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attempts_left: Option<i64>,
}

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub retry_after: Option<i64>,
    pub attempts_left: Option<i64>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        ApiError {
            status,
            code,
            message: message.into(),
            retry_after: None,
            attempts_left: None,
        }
    }

    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", "Sign in first.")
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", message)
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }
}

impl From<CoreError> for ApiError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::NotFound => ApiError::new(StatusCode::NOT_FOUND, "not_found", "Not found."),
            CoreError::Conflict(m) => ApiError::conflict(m),
            CoreError::Invalid(m) => ApiError::new(StatusCode::BAD_REQUEST, "invalid", m),
            CoreError::Forbidden(m) => ApiError::forbidden(m),
            CoreError::BadSecret { attempts_left } => ApiError {
                attempts_left: Some(attempts_left),
                ..ApiError::new(StatusCode::UNAUTHORIZED, "bad_secret", "Wrong PIN or password.")
            },
            CoreError::Locked { retry_after } => ApiError {
                retry_after: Some(retry_after),
                ..ApiError::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "locked",
                    format!("Too many wrong attempts. Try again in {retry_after} seconds."),
                )
            },
            other @ (CoreError::Db(_) | CoreError::Migrate(_) | CoreError::Internal(_)) => {
                tracing::error!(error = %other, "internal error");
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "Something went wrong inside Outpost.",
                )
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            error: ErrorDetail {
                code: self.code.to_string(),
                message: self.message,
                retry_after: self.retry_after,
                attempts_left: self.attempts_left,
            },
        };
        let mut response = (self.status, Json(body)).into_response();
        if let Some(secs) = self.retry_after {
            if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, v);
            }
        }
        response
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

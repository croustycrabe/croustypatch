use actix_web::{http::StatusCode, HttpResponse, ResponseError};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("authentication required")]
    Unauthorized,
    #[error("{0}")]
    Forbidden(String),
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Conflict(String),
    #[error("too many attempts, try again later")]
    TooManyRequests,
    #[error("internal error")]
    Internal(String),
}

impl ApiError {
    pub fn bad(m: impl Into<String>) -> Self {
        Self::BadRequest(m.into())
    }
    pub fn forbidden(m: impl Into<String>) -> Self {
        Self::Forbidden(m.into())
    }
    pub fn internal(e: impl std::fmt::Display) -> Self {
        Self::Internal(e.to_string())
    }
}

impl ResponseError for ApiError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn error_response(&self) -> HttpResponse {
        if let Self::Internal(detail) = self {
            // The client never sees internals.
            tracing::error!(%detail, "internal error");
        }
        let code = match self {
            Self::BadRequest(_) => "bad_request",
            Self::Unauthorized => "unauthorized",
            Self::Forbidden(_) => "forbidden",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::TooManyRequests => "too_many_requests",
            Self::Internal(_) => "internal",
        };
        HttpResponse::build(self.status_code()).json(json!({ "error": code, "message": self.to_string() }))
    }
}

impl From<diesel::result::Error> for ApiError {
    fn from(e: diesel::result::Error) -> Self {
        use diesel::result::{DatabaseErrorKind, Error};
        match e {
            Error::NotFound => ApiError::NotFound("resource"),
            Error::DatabaseError(DatabaseErrorKind::UniqueViolation, info) => ApiError::Conflict(format!("already exists ({})", info.constraint_name().unwrap_or("unique"))),
            Error::DatabaseError(DatabaseErrorKind::ForeignKeyViolation, _) => ApiError::bad("referenced item does not exist or is still in use"),
            other => ApiError::Internal(other.to_string()),
        }
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

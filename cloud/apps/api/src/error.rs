use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use uuid::Uuid;
use zene_cloud_domain::ApiError;

pub struct AppError {
    status: StatusCode,
    error: String,
    message: String,
    retryable: bool,
}

impl AppError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            error: "bad_request".into(),
            message: message.into(),
            retryable: false,
        }
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            error: "unauthorized".into(),
            message: message.into(),
            retryable: false,
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            error: "forbidden".into(),
            message: message.into(),
            retryable: false,
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            error: "not_found".into(),
            message: message.into(),
            retryable: false,
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            error: "conflict".into(),
            message: message.into(),
            retryable: false,
        }
    }

    pub fn stale_attempt(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            error: "stale_attempt".into(),
            message: message.into(),
            retryable: false,
        }
    }

    pub fn internal(err: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            error: "internal_error".into(),
            message: err.to_string(),
            retryable: true,
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(value: anyhow::Error) -> Self {
        let msg = value.to_string();
        if msg.contains("invalid credentials") || msg.contains("password") {
            return Self::unauthorized(msg);
        }
        if msg.contains("not found") {
            return Self::not_found(msg);
        }
        if msg.contains("stale_attempt") {
            return Self::stale_attempt(msg);
        }
        if msg.contains("already exist") {
            return Self::conflict(msg);
        }
        Self::internal(msg)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let body = ApiError {
            error: self.error,
            message: self.message,
            retryable: self.retryable,
            trace_id: Uuid::new_v4().to_string(),
        };
        (self.status, Json(body)).into_response()
    }
}

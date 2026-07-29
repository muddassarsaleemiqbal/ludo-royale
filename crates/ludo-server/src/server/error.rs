//! Stable API error mapping. Database details are logged but never leaked.

use super::{IntoResponse, Json, StatusCode};

pub(super) struct ApiError(pub(super) StatusCode, pub(super) String);

impl ApiError {
    pub(super) fn bad_request(message: &str) -> Self {
        Self(StatusCode::BAD_REQUEST, message.to_owned())
    }

    pub(super) fn unauthorized(message: &str) -> Self {
        Self(StatusCode::UNAUTHORIZED, message.to_owned())
    }

    pub(super) fn conflict(message: &str) -> Self {
        Self(StatusCode::CONFLICT, message.to_owned())
    }

    pub(super) fn internal(message: &str) -> Self {
        Self(StatusCode::INTERNAL_SERVER_ERROR, message.to_owned())
    }

    pub(super) fn too_many_requests(message: &str) -> Self {
        Self(StatusCode::TOO_MANY_REQUESTS, message.to_owned())
    }

    pub(super) fn code(&self) -> &'static str {
        match self.1.as_str() {
            "Game not found" => "game_not_found",
            "The game advanced; refreshing the board" => "stale_revision",
            "This table is full" => "lobby_full",
            "Only the host can start this game" => "host_required",
            _ if self.0 == StatusCode::UNAUTHORIZED => "unauthorized",
            _ if self.0 == StatusCode::CONFLICT => "conflict",
            _ if self.0 == StatusCode::TOO_MANY_REQUESTS => "rate_limited",
            _ if self.0 == StatusCode::BAD_REQUEST => "invalid_request",
            _ => "internal_error",
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        tracing::error!(%error, "database operation failed");
        let conflict = error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .is_some_and(|code| matches!(code.as_ref(), "23505" | "40001" | "40P01"));
        if conflict {
            Self::conflict("The table changed while processing that request. Please try again.")
        } else {
            Self::internal("Database operation failed")
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

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

#[cfg(test)]
mod tests {
    use super::{ApiError, StatusCode};

    #[test]
    fn stable_game_errors_have_protocol_codes() {
        let cases = [
            (ApiError::bad_request("Game not found"), "game_not_found"),
            (
                ApiError::conflict("The game advanced; refreshing the board"),
                "stale_revision",
            ),
            (ApiError::conflict("This table is full"), "lobby_full"),
            (
                ApiError::unauthorized("Only the host can start this game"),
                "host_required",
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(error.code(), expected);
        }
    }

    #[test]
    fn status_fallbacks_are_safe_and_predictable() {
        assert_eq!(ApiError::unauthorized("No").code(), "unauthorized");
        assert_eq!(ApiError::conflict("Changed").code(), "conflict");
        assert_eq!(
            ApiError::too_many_requests("Slow down").code(),
            "rate_limited"
        );
        assert_eq!(ApiError::bad_request("Bad").code(), "invalid_request");
        assert_eq!(ApiError::internal("Failed").code(), "internal_error");
    }

    #[test]
    fn constructors_preserve_public_status_and_message() {
        let error = ApiError::bad_request("Invalid game options");
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
        assert_eq!(error.1, "Invalid game options");

        let error = ApiError::too_many_requests("Try later");
        assert_eq!(error.0, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(error.1, "Try later");
    }
}

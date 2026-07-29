//! Minimal authenticated operations surface with durable audit logging.

use super::{ApiError, AppState, HeaderMap, Json, Path, Row, State, StatusCode, Uuid};
use serde::Serialize;

#[derive(Serialize)]
pub(super) struct AdminOverview {
    users: i64,
    active_lobbies: i64,
    completed_matches: i64,
    pending_deletions: i64,
}

pub(super) async fn admin_overview(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AdminOverview>, ApiError> {
    authorize(&state, &headers)?;
    let row = sqlx::query(
        "SELECT
           (SELECT count(*) FROM users),
           (SELECT count(*) FROM game_lobbies WHERE status IN ('waiting','playing')),
           (SELECT count(*) FROM match_results),
           (SELECT count(*) FROM account_deletion_requests WHERE cancelled_at IS NULL)",
    )
    .fetch_one(&state.db)
    .await?;
    Ok(Json(AdminOverview {
        users: row.get(0),
        active_lobbies: row.get(1),
        completed_matches: row.get(2),
        pending_deletions: row.get(3),
    }))
}

pub(super) async fn admin_delete_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let actor = authorize(&state, &headers)?;
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "INSERT INTO admin_audit_log(actor,action,target_user_id)
         VALUES($1,'delete_user',$2)",
    )
    .bind(actor)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM users WHERE id=$1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

fn authorize<'a>(state: &AppState, headers: &'a HeaderMap) -> Result<&'a str, ApiError> {
    let configured = state
        .config
        .admin_token
        .as_deref()
        .ok_or_else(|| ApiError::unauthorized("Admin API is disabled"))?;
    let supplied = headers
        .get("x-ludo-admin-token")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::unauthorized("Admin authentication is required"))?;
    if supplied != configured {
        return Err(ApiError::unauthorized("Admin authentication failed"));
    }
    Ok("admin-api")
}

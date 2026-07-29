//! Account registration, session lifecycle, and credential validation.

use super::{
    ApiError, AppState, Argon2, AuthResponse, Credentials, HeaderMap, Json, OsRng, PasswordHash,
    PasswordHasher, PasswordVerifier, SESSION_SECONDS, SaltString, Sha256, State, StatusCode,
    SystemTime, UNIX_EPOCH, User, Uuid, enforce_rate_limit,
};
use sha2::Digest;

pub(super) async fn register(
    State(state): State<AppState>,
    Json(input): Json<Credentials>,
) -> Result<Json<AuthResponse>, ApiError> {
    let email = normalize_email(&input.email)?;
    enforce_rate_limit(&state, &format!("register:{email}"), 5).await?;
    validate_password(&input.password)?;
    let display_name = normalize_name(input.display_name.as_deref().unwrap_or(""))?;
    let password_hash = hash_password(&input.password)?;
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,$3,$4)")
        .bind(id)
        .bind(&email)
        .bind(&display_name)
        .bind(password_hash)
        .execute(&state.db)
        .await
        .map_err(|_| ApiError::conflict("An account with that email already exists"))?;
    issue_session(
        &state,
        User {
            id,
            email,
            display_name,
        },
    )
    .await
}

pub(super) async fn login(
    State(state): State<AppState>,
    Json(input): Json<Credentials>,
) -> Result<Json<AuthResponse>, ApiError> {
    let email = normalize_email(&input.email)?;
    enforce_rate_limit(&state, &format!("login:{email}"), 10).await?;
    let row: (Uuid, String, String, String) =
        sqlx::query_as("SELECT id,email,display_name,password_hash FROM users WHERE email=$1")
            .bind(email)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| ApiError::unauthorized("Invalid email or password"))?;
    let parsed = PasswordHash::new(&row.3)
        .map_err(|_| ApiError::internal("Stored password hash is invalid"))?;
    Argon2::default()
        .verify_password(input.password.as_bytes(), &parsed)
        .map_err(|_| ApiError::unauthorized("Invalid email or password"))?;
    issue_session(
        &state,
        User {
            id: row.0,
            email: row.1,
            display_name: row.2,
        },
    )
    .await
}

pub(super) async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<User>, ApiError> {
    Ok(Json(authenticate_header(&state, &headers).await?))
}

pub(super) async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    sqlx::query("DELETE FROM sessions WHERE token_hash=$1")
        .bind(token_hash(bearer(&headers)?))
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn request_deletion(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticate_header(&state, &headers).await?;
    sqlx::query(
        "INSERT INTO account_deletion_requests(user_id) VALUES($1)
         ON CONFLICT(user_id) DO UPDATE SET requested_at=CURRENT_TIMESTAMP,
           execute_after=CURRENT_TIMESTAMP+INTERVAL '24 hours',cancelled_at=NULL",
    )
    .bind(user.id)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::ACCEPTED)
}

pub(super) async fn cancel_deletion(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticate_header(&state, &headers).await?;
    sqlx::query(
        "UPDATE account_deletion_requests SET cancelled_at=CURRENT_TIMESTAMP WHERE user_id=$1",
    )
    .bind(user.id)
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn issue_session(
    state: &AppState,
    user: User,
) -> Result<Json<AuthResponse>, ApiError> {
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    sqlx::query("INSERT INTO sessions(token_hash,user_id,expires_at) VALUES($1,$2,$3)")
        .bind(token_hash(&token))
        .bind(user.id)
        .bind(now() + SESSION_SECONDS)
        .execute(&state.db)
        .await?;
    Ok(Json(AuthResponse { token, user }))
}

pub(super) async fn authenticate_header(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<User, ApiError> {
    authenticate_token(state, bearer(headers)?).await
}

pub(super) async fn authenticate_token(state: &AppState, token: &str) -> Result<User, ApiError> {
    let row:(Uuid,String,String)=sqlx::query_as("SELECT u.id,u.email,u.display_name FROM sessions s JOIN users u ON u.id=s.user_id WHERE s.token_hash=$1 AND s.expires_at>$2").bind(token_hash(token)).bind(now()).fetch_optional(&state.db).await?.ok_or_else(||ApiError::unauthorized("Login required"))?;
    Ok(User {
        id: row.0,
        email: row.1,
        display_name: row.2,
    })
}

pub(super) fn bearer(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| ApiError::unauthorized("Login required"))
}

pub(super) fn websocket_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get("sec-websocket-protocol")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| {
            value
                .split(',')
                .map(str::trim)
                .find(|value| *value != "ludo")
        })
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::unauthorized("Login required"))
}

pub(super) fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

pub(super) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

pub(super) fn normalize_email(value: &str) -> Result<String, ApiError> {
    let value = value.trim().to_ascii_lowercase();
    if value.len() > 254 || !value.contains('@') {
        return Err(ApiError::bad_request("Enter a valid email"));
    }
    Ok(value)
}

pub(super) fn normalize_name(value: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if !(2..=24).contains(&value.chars().count()) {
        return Err(ApiError::bad_request(
            "Display name must be 2–24 characters",
        ));
    }
    Ok(value.to_owned())
}

pub(super) fn validate_password(value: &str) -> Result<(), ApiError> {
    if value.len() < 10 || value.len() > 128 {
        return Err(ApiError::bad_request("Password must be 10–128 characters"));
    }
    Ok(())
}

pub(super) fn hash_password(value: &str) -> Result<String, ApiError> {
    Argon2::default()
        .hash_password(value.as_bytes(), &SaltString::generate(&mut OsRng))
        .map(|h| h.to_string())
        .map_err(|_| ApiError::internal("Password hashing failed"))
}

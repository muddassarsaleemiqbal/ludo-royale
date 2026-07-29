//! Cross-cutting validation, rate limiting, health, CORS, and domain parsing.

use super::{
    AllowOrigin, Any, ApiError, AppState, BotDifficulty, CorsLayer, Duration, Instant, PlayerId,
    RulePreset, ServerConfig, State,
};

pub(super) async fn health_live() -> &'static str {
    "ok"
}

pub(super) async fn health_ready(State(state): State<AppState>) -> Result<&'static str, ApiError> {
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.db)
        .await?;
    Ok("ready")
}

pub(super) async fn enforce_rate_limit(
    state: &AppState,
    key: &str,
    maximum_per_minute: usize,
) -> Result<(), ApiError> {
    let cutoff = Instant::now()
        .checked_sub(Duration::from_mins(1))
        .unwrap_or_else(Instant::now);
    let mut limits = state.rate_limits.lock().await;
    let attempts = limits.entry(key.to_owned()).or_default();
    while attempts.front().is_some_and(|attempt| *attempt < cutoff) {
        attempts.pop_front();
    }
    if attempts.len() >= maximum_per_minute {
        return Err(ApiError::too_many_requests(
            "Too many requests. Please wait a moment and try again.",
        ));
    }
    attempts.push_back(Instant::now());
    if limits.len() > 10_000 {
        limits.retain(|_, attempts| attempts.back().is_some_and(|attempt| *attempt >= cutoff));
    }
    Ok(())
}

pub(super) fn validate_lobby_options(preset: &str, difficulty: &str) -> Result<(), ApiError> {
    if !["classic", "quick", "tournament"].contains(&preset)
        || !["easy", "medium", "hard"].contains(&difficulty)
    {
        return Err(ApiError::bad_request("Invalid game options"));
    }
    Ok(())
}

pub(super) fn parse_preset(value: &str) -> RulePreset {
    match value {
        "quick" => RulePreset::Quick,
        "tournament" => RulePreset::Tournament,
        _ => RulePreset::Classic,
    }
}

pub(super) fn parse_difficulty(value: &str) -> BotDifficulty {
    match value {
        "easy" => BotDifficulty::Easy,
        "hard" => BotDifficulty::Hard,
        _ => BotDifficulty::Medium,
    }
}

pub(super) fn player_id(index: u8) -> PlayerId {
    PlayerId::new(index).unwrap_or_else(|| std::process::abort())
}

pub(super) fn cors_layer(config: &ServerConfig) -> Result<CorsLayer, Box<dyn std::error::Error>> {
    if config.allowed_origins.is_empty() {
        tracing::warn!(
            "LUDO_ALLOWED_ORIGINS is unset; allowing all origins for backward compatibility"
        );
        return Ok(CorsLayer::permissive());
    }
    let mut origin_values = config.allowed_origins.clone();
    for desktop_origin in [
        "tauri://localhost",
        "http://tauri.localhost",
        "https://tauri.localhost",
    ] {
        if !origin_values.iter().any(|origin| origin == desktop_origin) {
            origin_values.push(desktop_origin.to_owned());
        }
    }
    let origins = origin_values
        .iter()
        .map(String::as_str)
        .map(str::parse)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods(Any)
        .allow_headers(Any))
}

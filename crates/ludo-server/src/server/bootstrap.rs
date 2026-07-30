//! Process bootstrap, dependency construction, routing, and graceful shutdown.

use super::{
    AblyConfig, AppState, Arc, HashMap, HeaderName, MakeRequestUuid, Metrics, Mutex, PgPoolOptions,
    PropagateRequestIdLayer, Router, RwLock, ServerConfig, SetRequestIdLayer, TraceLayer,
    ably_token, admin_delete_user, admin_overview, cancel_deletion, clear_all_data, cors_layer,
    delete, env, get, health_live, health_ready, login, logout, me, metrics, post, register,
    request_deletion, run_match_supervisor, run_outbox, websocket,
};

pub(crate) async fn run() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = Arc::new(ServerConfig::from_env()?);
    let db = PgPoolOptions::new()
        .max_connections(config.max_db_connections)
        .min_connections(config.min_db_connections)
        .acquire_timeout(config.db_acquire_timeout)
        .idle_timeout(config.db_idle_timeout)
        .connect(&config.database_url)
        .await?;
    sqlx::migrate!().run(&db).await?;
    sqlx::query("DELETE FROM sessions WHERE expires_at<$1")
        .bind(super::now())
        .execute(&db)
        .await?;

    let state = AppState {
        db,
        sockets: Arc::new(Mutex::new(HashMap::new())),
        ably: env::var("ABLY_API_KEY").ok().and_then(|key| {
            key.split_once(':')
                .map(|(key_name, key_secret)| AblyConfig {
                    key_name: key_name.to_owned(),
                    key_secret: key_secret.to_owned(),
                    http: reqwest::Client::new(),
                })
        }),
        rate_limits: Arc::new(Mutex::new(HashMap::new())),
        alert_webhook: env::var("LUDO_ALERT_WEBHOOK").ok(),
        config,
        metrics: Arc::new(Metrics::default()),
        leaderboard_cache: Arc::new(RwLock::new(None)),
    };

    if state.ably.is_some() {
        tokio::spawn(run_outbox(state.clone()));
    }
    tokio::spawn(run_match_supervisor(state.clone()));

    let address = state.config.address;
    let app = router(state)?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "online server listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

fn router(state: AppState) -> Result<Router, Box<dyn std::error::Error>> {
    let request_id = HeaderName::from_static("x-request-id");
    Ok(Router::new()
        .route("/health", get(health_live))
        .route("/health/ready", get(health_ready))
        .route("/metrics", get(metrics))
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/me", get(me))
        .route(
            "/api/me/deletion",
            post(request_deletion).delete(cancel_deletion),
        )
        .route("/api/admin/overview", get(admin_overview))
        .route("/api/admin/users/{user_id}", delete(admin_delete_user))
        .route("/api/admin/clear-all", delete(clear_all_data))
        .route("/api/ably/token", get(ably_token))
        .route("/api/online", get(websocket))
        .layer(cors_layer(&state.config)?)
        .layer(PropagateRequestIdLayer::new(request_id.clone()))
        .layer(SetRequestIdLayer::new(request_id, MakeRequestUuid))
        .layer(TraceLayer::new_for_http())
        .with_state(state))
}

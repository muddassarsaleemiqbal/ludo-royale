//! Websocket command transport, idempotency, and optional Ably delivery.

use super::{
    AblyClaims, AblyConfig, Algorithm, ApiError, AppState, ClientEnvelope, ClientMessage, Duration,
    EncodingKey, Header, HeaderMap, IntoResponse, Message, Ordering, PgPool, Postgres, Row,
    Serialize, ServerMessage, SinkExt, State, StatusCode, Transaction, User, Uuid, WebSocket,
    WebSocketUpgrade, add_activity, apply_action, authenticate_header, authenticate_token,
    broadcast_lobbies, broadcast_presence, create_lobby, encode, end_game, enforce_rate_limit,
    invite_friend, kick_player, leave_lobby, leave_match, mpsc, now, quick_match, ranked_match,
    remove_friend, request_join, respond_friend_invite, respond_friend_request, respond_join,
    resume_user_state, search_players, send_friend_request, send_hub, send_lobbies, send_lobby,
    send_lobby_to, send_replay, set_cosmetics, spectate, start_game, sync_game, update_lobby,
    vote_rematch, websocket_token,
};
use futures_util::StreamExt;

pub(super) async fn websocket(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<impl IntoResponse, ApiError> {
    let token = websocket_token(&headers)?;
    let user = authenticate_token(&state, token).await?;
    Ok(upgrade
        .protocols(["ludo"])
        .on_upgrade(move |socket| online_socket(state, user, socket)))
}

pub(super) async fn ably_token(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<String, ApiError> {
    let user = authenticate_header(&state, &headers).await?;
    let ably = state
        .ably
        .as_ref()
        .ok_or_else(|| ApiError::internal("Ably is not configured"))?;
    create_ably_jwt(ably, user.id)
}

pub(super) fn create_ably_jwt(ably: &AblyConfig, user_id: Uuid) -> Result<String, ApiError> {
    let capability = serde_json::json!({
        format!("ludo:user:{user_id}"): ["subscribe"],
        "ludo:lobbies": ["subscribe"]
    })
    .to_string();
    let issued = now();
    let claims = AblyClaims {
        iat: issued,
        exp: issued + 60 * 60,
        capability,
        client_id: user_id.to_string(),
    };
    let mut header = Header::new(Algorithm::HS256);
    header.kid = Some(ably.key_name.clone());
    encode(
        &header,
        &claims,
        &EncodingKey::from_secret(ably.key_secret.as_bytes()),
    )
    .map_err(|_| ApiError::internal("Could not create Ably token"))
}

pub(super) async fn online_socket(state: AppState, user: User, socket: WebSocket) {
    let (mut sink, mut source) = socket.split();
    let (sender, mut receiver) = mpsc::channel(state.config.outbound_queue_capacity);
    state.sockets.lock().await.insert(user.id, sender.clone());
    state
        .metrics
        .websocket_connections_total
        .fetch_add(1, Ordering::Relaxed);
    state
        .metrics
        .websocket_connections_active
        .fetch_add(1, Ordering::Relaxed);
    let heartbeat = spawn_presence_heartbeat(state.db.clone(), user.id);
    let _ = sender.try_send(ServerMessage::Ready {
        user: user.clone(),
        protocol_version: state.config.protocol_version,
    });
    let writer = tokio::spawn(async move {
        while let Some(message) = receiver.recv().await {
            let Ok(text) = serde_json::to_string(&message) else {
                continue;
            };
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });
    send_lobbies(&state, &user).await;
    let _ = send_hub(&state, &user).await;
    resume_user_state(&state, &user).await;
    while let Some(Ok(Message::Text(text))) = source.next().await {
        if text.len() > 16 * 1024 {
            send_to(
                &state,
                user.id,
                ServerMessage::Error {
                    command_id: None,
                    code: "message_too_large",
                    message: "Online messages are limited to 16 KiB".to_owned(),
                    recoverable: true,
                },
            )
            .await;
            continue;
        }
        match serde_json::from_str::<ClientEnvelope>(&text) {
            Ok(envelope) => handle_envelope(&state, &user, envelope).await,
            Err(_) => {
                send_to(
                    &state,
                    user.id,
                    ServerMessage::Error {
                        command_id: None,
                        code: "invalid_message",
                        message: "Invalid online message".to_owned(),
                        recoverable: true,
                    },
                )
                .await;
            }
        }
    }
    state.sockets.lock().await.remove(&user.id);
    state
        .metrics
        .websocket_connections_active
        .fetch_sub(1, Ordering::Relaxed);
    let lobbies: Vec<Uuid> = sqlx::query_scalar(
        "SELECT lobby_id FROM lobby_members WHERE user_id=$1
         UNION SELECT lobby_id FROM lobby_spectators WHERE user_id=$1",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    for lobby_id in lobbies {
        let _ = broadcast_presence(&state, lobby_id).await;
    }
    heartbeat.abort();
    writer.abort();
}

fn spawn_presence_heartbeat(db: PgPool, user_id: Uuid) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let _ = sqlx::query(
                "INSERT INTO user_presence(user_id,last_seen_at) VALUES($1,CURRENT_TIMESTAMP)
                 ON CONFLICT(user_id) DO UPDATE SET last_seen_at=CURRENT_TIMESTAMP",
            )
            .bind(user_id)
            .execute(&db)
            .await;
            let _ = sqlx::query(
                "UPDATE lobby_members SET last_seen_at=CURRENT_TIMESTAMP WHERE user_id=$1",
            )
            .bind(user_id)
            .execute(&db)
            .await;
            let _ = sqlx::query(
                "UPDATE lobby_spectators SET last_seen_at=CURRENT_TIMESTAMP WHERE user_id=$1",
            )
            .bind(user_id)
            .execute(&db)
            .await;
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    })
}

pub(super) async fn handle_envelope(state: &AppState, user: &User, envelope: ClientEnvelope) {
    state.metrics.commands_total.fetch_add(1, Ordering::Relaxed);
    if envelope
        .protocol_version
        .is_some_and(|version| version != state.config.protocol_version)
    {
        state
            .metrics
            .command_errors_total
            .fetch_add(1, Ordering::Relaxed);
        send_command_error(
            state,
            user.id,
            envelope.command_id,
            ApiError::conflict("This client version is incompatible with the server"),
        )
        .await;
        return;
    }
    let result = enforce_rate_limit(state, &format!("command:{}", user.id), 90).await;
    if let Err(error) = result {
        send_command_error(state, user.id, envelope.command_id, error).await;
        return;
    }
    if command_was_processed(state, user.id, envelope.command_id).await {
        send_to(
            state,
            user.id,
            ServerMessage::Ack {
                command_id: envelope.command_id,
            },
        )
        .await;
        return;
    }
    if let Err(error) = handle_online(state, user, envelope.message).await {
        state
            .metrics
            .command_errors_total
            .fetch_add(1, Ordering::Relaxed);
        tracing::warn!(
            user_id = %user.id,
            status = %error.0,
            message = %error.1,
            "online command failed"
        );
        send_command_error(state, user.id, envelope.command_id, error).await;
    } else {
        record_processed_command(state, user.id, envelope.command_id).await;
        send_to(
            state,
            user.id,
            ServerMessage::Ack {
                command_id: envelope.command_id,
            },
        )
        .await;
    }
}

pub(super) async fn send_command_error(
    state: &AppState,
    user_id: Uuid,
    command_id: Uuid,
    error: ApiError,
) {
    send_to(
        state,
        user_id,
        ServerMessage::Error {
            command_id: Some(command_id),
            code: error.code(),
            recoverable: error.0 != StatusCode::UNAUTHORIZED,
            message: error.1,
        },
    )
    .await;
}

pub(super) async fn command_was_processed(
    state: &AppState,
    user_id: Uuid,
    command_id: Uuid,
) -> bool {
    match sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM processed_commands WHERE command_id=$1 AND user_id=$2)",
    )
    .bind(command_id)
    .bind(user_id)
    .fetch_one(&state.db)
    .await
    {
        Ok(processed) => processed,
        Err(error) => {
            tracing::error!(%error, %user_id, %command_id, "could not check command replay");
            false
        }
    }
}

pub(super) async fn record_processed_command(state: &AppState, user_id: Uuid, command_id: Uuid) {
    if let Err(error) = sqlx::query(
        "INSERT INTO processed_commands(command_id,user_id) VALUES($1,$2)
         ON CONFLICT(command_id) DO NOTHING",
    )
    .bind(command_id)
    .bind(user_id)
    .execute(&state.db)
    .await
    {
        tracing::error!(%error, %user_id, %command_id, "could not record processed command");
    }
}

#[allow(clippy::too_many_lines)]
pub(super) async fn handle_online(
    state: &AppState,
    user: &User,
    message: ClientMessage,
) -> Result<(), ApiError> {
    match message {
        ClientMessage::Ping => send_to(state, user.id, ServerMessage::Pong).await,
        ClientMessage::ListLobbies => send_lobbies(state, user).await,
        ClientMessage::GetHub => {
            require_feature(state.config.flags.social, "Social features")?;
            send_hub(state, user).await?;
        }
        ClientMessage::SearchPlayers { query } => {
            require_feature(state.config.flags.social, "Social features")?;
            search_players(state, user, &query).await?;
        }
        ClientMessage::SendFriendRequest { user_id } => {
            require_feature(state.config.flags.social, "Social features")?;
            send_friend_request(state, user, user_id).await?;
        }
        ClientMessage::RespondFriendRequest { user_id, accept } => {
            require_feature(state.config.flags.social, "Social features")?;
            respond_friend_request(state, user, user_id, accept).await?;
        }
        ClientMessage::RemoveFriend { user_id } => {
            require_feature(state.config.flags.social, "Social features")?;
            remove_friend(state, user, user_id).await?;
        }
        ClientMessage::InviteFriend { lobby_id, user_id } => {
            require_feature(state.config.flags.social, "Social features")?;
            invite_friend(state, user, lobby_id, user_id).await?;
        }
        ClientMessage::RespondFriendInvite { invite_id, accept } => {
            require_feature(state.config.flags.social, "Social features")?;
            respond_friend_invite(state, user, invite_id, accept).await?;
        }
        ClientMessage::SetCosmetics {
            dice_theme,
            token_theme,
        } => set_cosmetics(state, user, &dice_theme, &token_theme).await?,
        ClientMessage::GetReplay { match_id } => {
            require_feature(state.config.flags.replays, "Replays")?;
            send_replay(state, user, match_id).await?;
        }
        ClientMessage::RankedMatch => {
            require_feature(state.config.flags.ranked, "Ranked play")?;
            ranked_match(state, user).await?;
        }
        ClientMessage::CreateLobby {
            name,
            rule_preset,
            bot_difficulty,
            is_public,
            turn_seconds,
        } => {
            create_lobby(
                state,
                user,
                name,
                rule_preset,
                bot_difficulty,
                is_public,
                turn_seconds,
            )
            .await?;
        }
        ClientMessage::RequestJoin { lobby_id } => request_join(state, user, lobby_id).await?,
        ClientMessage::JoinByCode { invite_code } => {
            let lobby_id: Uuid = sqlx::query_scalar(
                "SELECT id FROM game_lobbies WHERE invite_code=upper($1) AND status='waiting'",
            )
            .bind(invite_code.trim())
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(|| ApiError::bad_request("Invite code is invalid or expired"))?;
            request_join(state, user, lobby_id).await?;
        }
        ClientMessage::CancelJoin { lobby_id } => {
            sqlx::query(
                "DELETE FROM lobby_join_requests
                 WHERE lobby_id=$1 AND user_id=$2 AND status='pending'",
            )
            .bind(lobby_id)
            .bind(user.id)
            .execute(&state.db)
            .await?;
            if let Some(host) =
                sqlx::query_scalar("SELECT host_user_id FROM game_lobbies WHERE id=$1")
                    .bind(lobby_id)
                    .fetch_optional(&state.db)
                    .await?
            {
                send_lobby_to(state, lobby_id, host).await?;
            }
            broadcast_lobbies(state).await;
        }
        ClientMessage::RespondJoin { request_id, accept } => {
            respond_join(state, user, request_id, accept).await?;
        }
        ClientMessage::LeaveLobby { lobby_id } => leave_lobby(state, user, lobby_id).await?,
        ClientMessage::LeaveMatch { lobby_id } => leave_match(state, user, lobby_id).await?,
        ClientMessage::EndGame { lobby_id } => end_game(state, user, lobby_id).await?,
        ClientMessage::KickPlayer { lobby_id, user_id } => {
            kick_player(state, user, lobby_id, user_id).await?;
        }
        ClientMessage::SetReady { lobby_id, ready } => {
            sqlx::query(
                "UPDATE lobby_members SET ready=$3,last_seen_at=CURRENT_TIMESTAMP
                 WHERE lobby_id=$1 AND user_id=$2
                   AND EXISTS(SELECT 1 FROM game_lobbies WHERE id=$1 AND status='waiting')",
            )
            .bind(lobby_id)
            .bind(user.id)
            .bind(ready)
            .execute(&state.db)
            .await?;
            send_lobby(state, lobby_id).await?;
        }
        ClientMessage::UpdateLobby {
            lobby_id,
            rule_preset,
            bot_difficulty,
            is_public,
            turn_seconds,
            rematch_mode,
        } => {
            update_lobby(
                state,
                user,
                lobby_id,
                &rule_preset,
                &bot_difficulty,
                is_public,
                turn_seconds,
                &rematch_mode,
            )
            .await?;
        }
        ClientMessage::QuickMatch {
            rule_preset,
            bot_difficulty,
        } => quick_match(state, user, &rule_preset, &bot_difficulty).await?,
        ClientMessage::Spectate { lobby_id } => spectate(state, user, lobby_id).await?,
        ClientMessage::Chat { lobby_id, body } => {
            add_activity(state, user, lobby_id, "chat", &body).await?;
        }
        ClientMessage::React { lobby_id, emoji } => {
            add_activity(state, user, lobby_id, "reaction", &emoji).await?;
        }
        ClientMessage::VoteRematch { lobby_id } => vote_rematch(state, user, lobby_id).await?,
        ClientMessage::StartGame { lobby_id } => start_game(state, user, lobby_id).await?,
        ClientMessage::Sync { lobby_id } => sync_game(state, user, lobby_id).await?,
        ClientMessage::Roll { lobby_id, revision } => {
            apply_action(state, user, lobby_id, revision, None).await?;
        }
        ClientMessage::Move {
            lobby_id,
            revision,
            token,
        } => apply_action(state, user, lobby_id, revision, Some(token)).await?,
    }
    Ok(())
}

fn require_feature(enabled: bool, name: &str) -> Result<(), ApiError> {
    if enabled {
        Ok(())
    } else {
        Err(ApiError::conflict(&format!(
            "{name} is temporarily unavailable"
        )))
    }
}

pub(super) async fn send_to(state: &AppState, user: Uuid, message: ServerMessage) {
    if publish_ably(state, &format!("ludo:user:{user}"), "event", &message).await {
        return;
    }
    if let Some(socket) = state.sockets.lock().await.get(&user)
        && socket.try_send(message).is_err()
    {
        state
            .metrics
            .outbound_dropped_total
            .fetch_add(1, Ordering::Relaxed);
        tracing::warn!(%user, "dropping realtime message for slow client");
    }
}

pub(super) async fn publish_ably<T: Serialize + ?Sized>(
    state: &AppState,
    channel: &str,
    name: &str,
    data: &T,
) -> bool {
    let Some(ably) = &state.ably else {
        return false;
    };
    let response = ably
        .http
        .post(format!("https://rest.ably.io/channels/{channel}/messages"))
        .basic_auth(&ably.key_name, Some(&ably.key_secret))
        .json(&serde_json::json!({ "name": name, "data": data }))
        .send()
        .await;
    match response {
        Ok(response) if response.status().is_success() => true,
        Ok(response) => {
            tracing::warn!(status = %response.status(), %channel, "Ably rejected publish");
            false
        }
        Err(error) => {
            tracing::warn!(%error, %channel, "Ably publish failed");
            false
        }
    }
}

pub(super) async fn enqueue_outbox<T: Serialize + ?Sized>(
    tx: &mut Transaction<'_, Postgres>,
    channel: &str,
    event_name: &str,
    data: &T,
) -> Result<(), ApiError> {
    let payload =
        serde_json::to_value(data).map_err(|_| ApiError::internal("Could not queue update"))?;
    sqlx::query("INSERT INTO realtime_outbox(channel,event_name,payload) VALUES($1,$2,$3)")
        .bind(channel)
        .bind(event_name)
        .bind(payload)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(super) async fn run_outbox(state: AppState) {
    let mut cleanup_counter = 0_u16;
    loop {
        let rows = sqlx::query(
            "UPDATE realtime_outbox
             SET attempts=attempts+1,next_attempt_at=CURRENT_TIMESTAMP+INTERVAL '30 seconds'
             WHERE id IN (
               SELECT id FROM realtime_outbox
               WHERE published_at IS NULL AND next_attempt_at<=CURRENT_TIMESTAMP
               ORDER BY id FOR UPDATE SKIP LOCKED LIMIT 50
             )
             RETURNING id,channel,event_name,payload,attempts",
        )
        .fetch_all(&state.db)
        .await;
        match rows {
            Ok(rows) => {
                for row in rows {
                    let id: i64 = row.get(0);
                    let channel: String = row.get(1);
                    let event_name: String = row.get(2);
                    let payload: serde_json::Value = row.get(3);
                    let attempts: i32 = row.get(4);
                    let published = publish_ably(&state, &channel, &event_name, &payload).await;
                    if published
                        && let Err(error) = sqlx::query(
                            "UPDATE realtime_outbox SET published_at=CURRENT_TIMESTAMP WHERE id=$1",
                        )
                        .bind(id)
                        .execute(&state.db)
                        .await
                    {
                        tracing::error!(%error, outbox_id=id, "could not complete outbox event");
                    }
                    if !published && attempts == 5 {
                        send_deployment_alert(
                            &state,
                            "Ably delivery is failing",
                            &format!("Outbox event {id} failed five times on channel {channel}"),
                        )
                        .await;
                    }
                }
            }
            Err(error) => tracing::error!(%error, "could not claim realtime outbox events"),
        }
        cleanup_counter = cleanup_counter.wrapping_add(1);
        if cleanup_counter == 0 {
            if let Err(error) = sqlx::query(
                "DELETE FROM processed_commands
                 WHERE processed_at<CURRENT_TIMESTAMP-INTERVAL '24 hours'",
            )
            .execute(&state.db)
            .await
            {
                tracing::debug!(%error, "processed command cleanup deferred");
            }
            if let Err(error) = sqlx::query(
                "DELETE FROM realtime_outbox
                 WHERE published_at<CURRENT_TIMESTAMP-INTERVAL '7 days'",
            )
            .execute(&state.db)
            .await
            {
                tracing::debug!(%error, "outbox cleanup deferred");
            }
        }
        tokio::time::sleep(Duration::from_millis(750)).await;
    }
}

pub(super) async fn send_deployment_alert(state: &AppState, title: &str, detail: &str) {
    let Some(webhook) = &state.alert_webhook else {
        return;
    };
    if let Err(error) = reqwest::Client::new()
        .post(webhook)
        .json(&serde_json::json!({ "title": title, "detail": detail, "service": "ludo-server" }))
        .send()
        .await
    {
        tracing::warn!(%error, "could not send deployment alert");
    }
}

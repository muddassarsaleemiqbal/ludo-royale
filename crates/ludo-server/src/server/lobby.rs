//! Waiting-room lifecycle, seating, join approvals, and lobby discovery.

use super::{
    ApiError, AppState, JoinRequestView, LobbySummary, LobbyView, Postgres, Row, SeatView,
    ServerMessage, TURN_SECONDS, Transaction, User, Uuid, now, publish_ably, send_to, sync_game,
    validate_lobby_options,
};

pub(super) async fn create_lobby(
    state: &AppState,
    user: &User,
    name: String,
    rule_preset: String,
    bot_difficulty: String,
    is_public: bool,
    turn_seconds: u16,
) -> Result<(), ApiError> {
    validate_lobby_options(&rule_preset, &bot_difficulty)?;
    if ![15, 30, 45, 60].contains(&turn_seconds) {
        return Err(ApiError::bad_request("Invalid turn timer"));
    }
    let id = Uuid::new_v4();
    let invite_code = id.simple().to_string()[..8].to_ascii_uppercase();
    let name = if name.trim().is_empty() {
        format!("{}'s table", user.display_name)
    } else {
        name.trim().chars().take(40).collect()
    };
    let mut tx = state.db.begin().await?;
    let active_lobby: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM lobby_members m
            JOIN game_lobbies l ON l.id=m.lobby_id
            WHERE m.user_id=$1 AND l.status IN ('waiting','playing')
        )",
    )
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    if active_lobby {
        return Err(ApiError::conflict(
            "Leave or finish your current table before creating another",
        ));
    }
    sqlx::query("INSERT INTO game_lobbies(id,host_user_id,name,rule_preset,bot_difficulty,is_public,invite_code,turn_seconds) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(id).bind(user.id).bind(name).bind(rule_preset).bind(bot_difficulty).bind(is_public).bind(invite_code).bind(i16::try_from(turn_seconds).unwrap_or(30)).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO lobby_members(lobby_id,user_id,seat,ready) VALUES($1,$2,0,TRUE)")
        .bind(id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    send_lobby(state, id).await?;
    broadcast_lobbies(state).await;
    Ok(())
}

pub(super) async fn request_join(
    state: &AppState,
    user: &User,
    lobby_id: Uuid,
) -> Result<(), ApiError> {
    let inserted = sqlx::query(
        "INSERT INTO lobby_join_requests(id,lobby_id,user_id)
         SELECT $1,$2,$3
         WHERE EXISTS(
            SELECT 1 FROM game_lobbies l
            WHERE l.id=$2 AND l.status='waiting' AND l.host_user_id<>$3
              AND NOT EXISTS(
                SELECT 1 FROM lobby_members m WHERE m.lobby_id=l.id AND m.user_id=$3
              )
              AND (SELECT count(*) FROM lobby_members m WHERE m.lobby_id=l.id) < 4
         )
         ON CONFLICT(lobby_id,user_id)
         DO UPDATE SET status='pending',created_at=CURRENT_TIMESTAMP",
    )
    .bind(Uuid::new_v4())
    .bind(lobby_id)
    .bind(user.id)
    .execute(&state.db)
    .await?;
    if inserted.rows_affected() == 0 {
        return Err(ApiError::conflict(
            "This table is unavailable, full, or you already have a seat",
        ));
    }
    let host: Option<Uuid> =
        sqlx::query_scalar("SELECT host_user_id FROM game_lobbies WHERE id=$1")
            .bind(lobby_id)
            .fetch_optional(&state.db)
            .await?;
    if let Some(host) = host {
        send_lobby_to(state, lobby_id, host).await?;
    }
    send_to(state, user.id, ServerMessage::JoinRequested { lobby_id }).await;
    broadcast_lobbies(state).await;
    Ok(())
}

pub(super) async fn respond_join(
    state: &AppState,
    user: &User,
    request_id: Uuid,
    accept: bool,
) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await?;
    let row = sqlx::query("SELECT r.lobby_id,r.user_id FROM lobby_join_requests r JOIN game_lobbies l ON l.id=r.lobby_id WHERE r.id=$1 AND l.host_user_id=$2 AND l.status='waiting' AND r.status='pending' FOR UPDATE OF l,r")
        .bind(request_id).bind(user.id).fetch_optional(&mut *tx).await?.ok_or_else(|| ApiError::bad_request("Join request is no longer available"))?;
    let lobby_id: Uuid = row.get(0);
    let joining: Uuid = row.get(1);
    if accept {
        accept_joining_player(&mut tx, lobby_id, joining).await?;
    }
    sqlx::query("UPDATE lobby_join_requests SET status=$2::join_request_status WHERE id=$1")
        .bind(request_id)
        .bind(if accept { "accepted" } else { "declined" })
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    send_lobby(state, lobby_id).await?;
    send_to(
        state,
        joining,
        ServerMessage::JoinDecision {
            lobby_id,
            accepted: accept,
        },
    )
    .await;
    broadcast_lobbies(state).await;
    Ok(())
}

pub(super) async fn accept_joining_player(
    tx: &mut Transaction<'_, Postgres>,
    lobby_id: Uuid,
    joining: Uuid,
) -> Result<(), ApiError> {
    let existing: Option<i16> =
        sqlx::query_scalar("SELECT seat FROM lobby_members WHERE lobby_id=$1 AND user_id=$2")
            .bind(lobby_id)
            .bind(joining)
            .fetch_optional(&mut **tx)
            .await?;
    let active_elsewhere: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM lobby_members m JOIN game_lobbies l ON l.id=m.lobby_id
            WHERE m.user_id=$1 AND m.lobby_id<>$2 AND l.status IN ('waiting','playing')
        )",
    )
    .bind(joining)
    .bind(lobby_id)
    .fetch_one(&mut **tx)
    .await?;
    if active_elsewhere {
        return Err(ApiError::conflict(
            "That player has already joined another active table",
        ));
    }
    if existing.is_some() {
        return Ok(());
    }
    let seat: Option<i16> = sqlx::query_scalar(
        "SELECT s::smallint FROM generate_series(1,3) AS s
         WHERE NOT EXISTS(
            SELECT 1 FROM lobby_members WHERE lobby_id=$1 AND seat=s::smallint
         )
         ORDER BY s LIMIT 1",
    )
    .bind(lobby_id)
    .fetch_optional(&mut **tx)
    .await?;
    let seat = seat.ok_or_else(|| ApiError::conflict("This table is full"))?;
    sqlx::query("INSERT INTO lobby_members(lobby_id,user_id,seat) VALUES($1,$2,$3)")
        .bind(lobby_id)
        .bind(joining)
        .bind(seat)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(super) async fn leave_lobby(
    state: &AppState,
    user: &User,
    lobby_id: Uuid,
) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await?;
    let lobby: Option<Uuid> = sqlx::query_scalar(
        "SELECT host_user_id FROM game_lobbies
         WHERE id=$1 AND status='waiting' FOR UPDATE",
    )
    .bind(lobby_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(host_id) = lobby else {
        sqlx::query("DELETE FROM lobby_spectators WHERE lobby_id=$1 AND user_id=$2")
            .bind(lobby_id)
            .bind(user.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok(());
    };
    sqlx::query("DELETE FROM lobby_members WHERE lobby_id=$1 AND user_id=$2")
        .bind(lobby_id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    if host_id == user.id {
        let successor: Option<Uuid> = sqlx::query_scalar(
            "SELECT user_id FROM lobby_members WHERE lobby_id=$1 ORDER BY joined_at LIMIT 1",
        )
        .bind(lobby_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(successor) = successor {
            sqlx::query(
                "UPDATE game_lobbies SET host_user_id=$2,updated_at=CURRENT_TIMESTAMP WHERE id=$1",
            )
            .bind(lobby_id)
            .bind(successor)
            .execute(&mut *tx)
            .await?;
            sqlx::query("UPDATE lobby_members SET ready=TRUE WHERE lobby_id=$1 AND user_id=$2")
                .bind(lobby_id)
                .bind(successor)
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query("DELETE FROM game_lobbies WHERE id=$1")
                .bind(lobby_id)
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    if sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM game_lobbies WHERE id=$1)")
        .bind(lobby_id)
        .fetch_one(&state.db)
        .await?
    {
        send_lobby(state, lobby_id).await?;
    }
    broadcast_lobbies(state).await;
    Ok(())
}

pub(super) async fn kick_player(
    state: &AppState,
    user: &User,
    lobby_id: Uuid,
    kicked_user_id: Uuid,
) -> Result<(), ApiError> {
    let removed = sqlx::query(
        "DELETE FROM lobby_members
         WHERE lobby_id=$1 AND user_id=$2 AND user_id<>$3
           AND EXISTS(SELECT 1 FROM game_lobbies
             WHERE id=$1 AND host_user_id=$3 AND status='waiting')",
    )
    .bind(lobby_id)
    .bind(kicked_user_id)
    .bind(user.id)
    .execute(&state.db)
    .await?;
    if removed.rows_affected() == 0 {
        return Err(ApiError::bad_request(
            "Only the host can remove that player",
        ));
    }
    send_to(
        state,
        kicked_user_id,
        ServerMessage::JoinDecision {
            lobby_id,
            accepted: false,
        },
    )
    .await;
    send_lobby(state, lobby_id).await?;
    broadcast_lobbies(state).await;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn update_lobby(
    state: &AppState,
    user: &User,
    lobby_id: Uuid,
    preset: &str,
    difficulty: &str,
    is_public: bool,
    turn_seconds: u16,
    rematch_mode: &str,
) -> Result<(), ApiError> {
    validate_lobby_options(preset, difficulty)?;
    if ![15, 30, 45, 60].contains(&turn_seconds) {
        return Err(ApiError::bad_request("Invalid turn timer"));
    }
    if !["vote", "host", "automatic"].contains(&rematch_mode) {
        return Err(ApiError::bad_request("Invalid rematch setting"));
    }
    let changed = sqlx::query(
        "UPDATE game_lobbies
         SET rule_preset=CASE WHEN ranked THEN rule_preset ELSE $3 END,
             bot_difficulty=CASE WHEN ranked THEN bot_difficulty ELSE $4 END,
             is_public=CASE WHEN ranked THEN is_public ELSE $5 END,
             turn_seconds=$6,rematch_mode=$7,
             updated_at=CURRENT_TIMESTAMP
         WHERE id=$1 AND host_user_id=$2 AND status='waiting'",
    )
    .bind(lobby_id)
    .bind(user.id)
    .bind(preset)
    .bind(difficulty)
    .bind(is_public)
    .bind(i16::try_from(turn_seconds).unwrap_or(30))
    .bind(rematch_mode)
    .execute(&state.db)
    .await?;
    if changed.rows_affected() == 0 {
        return Err(ApiError::bad_request(
            "Only the host can change table settings",
        ));
    }
    send_lobby(state, lobby_id).await?;
    broadcast_lobbies(state).await;
    Ok(())
}

pub(super) async fn quick_match(
    state: &AppState,
    user: &User,
    preset: &str,
    difficulty: &str,
) -> Result<(), ApiError> {
    validate_lobby_options(preset, difficulty)?;
    let mut tx = state.db.begin().await?;
    let lobby_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT l.id FROM game_lobbies l
         WHERE l.status='waiting' AND l.is_public AND l.rule_preset=$1
           AND l.bot_difficulty=$2 AND l.host_user_id<>$3
           AND (SELECT count(*) FROM lobby_members m WHERE m.lobby_id=l.id)<4
         ORDER BY l.created_at FOR UPDATE SKIP LOCKED LIMIT 1",
    )
    .bind(preset)
    .bind(difficulty)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(lobby_id) = lobby_id {
        accept_joining_player(&mut tx, lobby_id, user.id).await?;
        tx.commit().await?;
        send_lobby(state, lobby_id).await?;
        broadcast_lobbies(state).await;
    } else {
        tx.rollback().await?;
        create_lobby(
            state,
            user,
            "Quick Match".to_owned(),
            preset.to_owned(),
            difficulty.to_owned(),
            true,
            TURN_SECONDS,
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn send_lobbies(state: &AppState, user: &User) {
    let result = async {
        let rows = sqlx::query("SELECT l.id,l.name,u.display_name,(SELECT count(*) FROM lobby_members m WHERE m.lobby_id=l.id),l.rule_preset,l.bot_difficulty,l.status::text,l.host_user_id=$1,EXISTS(SELECT 1 FROM lobby_join_requests r WHERE r.lobby_id=l.id AND r.user_id=$1 AND r.status='pending') FROM game_lobbies l JOIN users u ON u.id=l.host_user_id WHERE (l.is_public OR l.host_user_id=$1 OR EXISTS(SELECT 1 FROM lobby_members m WHERE m.lobby_id=l.id AND m.user_id=$1)) AND l.status IN ('waiting','playing') ORDER BY l.updated_at DESC LIMIT 100")
            .bind(user.id).fetch_all(&state.db).await?;
        Ok::<_, sqlx::Error>(rows.into_iter().map(|r| LobbySummary { id:r.get(0),name:r.get(1),host_name:r.get(2),human_players:r.get(3),rule_preset:r.get(4),bot_difficulty:r.get(5),status:r.get(6),is_host:r.get(7),requested:r.get(8) }).collect())
    }.await;
    match result {
        Ok(lobbies) => send_to(state, user.id, ServerMessage::LobbyList { lobbies }).await,
        Err(_) => {
            send_to(
                state,
                user.id,
                ServerMessage::Error {
                    command_id: None,
                    code: "lobby_list_failed",
                    message: "Could not load games".to_owned(),
                    recoverable: true,
                },
            )
            .await;
        }
    }
}

pub(super) async fn resume_user_state(state: &AppState, user: &User) {
    let row = sqlx::query(
        "SELECT l.id,l.status::text
         FROM lobby_members m
         JOIN game_lobbies l ON l.id=m.lobby_id
         WHERE m.user_id=$1 AND l.status IN ('waiting','playing')
         ORDER BY l.updated_at DESC
         LIMIT 1",
    )
    .bind(user.id)
    .fetch_optional(&state.db)
    .await;
    match row {
        Ok(Some(row)) if row.get::<String, _>(1) == "playing" => {
            if let Err(error) = sync_game(state, user, row.get(0)).await {
                tracing::warn!(user_id=%user.id, message=%error.1, "failed to resume game");
            }
        }
        Ok(Some(row)) => {
            if let Err(error) = send_lobby_to(state, row.get(0), user.id).await {
                tracing::warn!(user_id=%user.id, message=%error.1, "failed to resume lobby");
            }
        }
        Ok(None) => {}
        Err(error) => tracing::error!(%error, user_id=%user.id, "failed to query resumable state"),
    }
}

pub(super) async fn broadcast_lobbies(state: &AppState) {
    if state.ably.is_some() {
        publish_ably(
            state,
            "ludo:lobbies",
            "changed",
            &serde_json::json!({ "at": now() }),
        )
        .await;
        return;
    }
    let users: Vec<Uuid> = state.sockets.lock().await.keys().copied().collect();
    for id in users {
        if let Ok(Some(row)) = sqlx::query_as::<_, (Uuid, String, String)>(
            "SELECT id,email,display_name FROM users WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(&state.db)
        .await
        {
            send_lobbies(
                state,
                &User {
                    id: row.0,
                    email: row.1,
                    display_name: row.2,
                },
            )
            .await;
        }
    }
}

pub(super) async fn send_lobby(state: &AppState, lobby_id: Uuid) -> Result<(), ApiError> {
    let users: Vec<Uuid> = sqlx::query_scalar("SELECT user_id FROM lobby_members WHERE lobby_id=$1 UNION SELECT host_user_id FROM game_lobbies WHERE id=$1").bind(lobby_id).fetch_all(&state.db).await?;
    for user in users {
        send_lobby_to(state, lobby_id, user).await?;
    }
    Ok(())
}

pub(super) async fn send_lobby_to(
    state: &AppState,
    lobby_id: Uuid,
    user_id: Uuid,
) -> Result<(), ApiError> {
    let row = sqlx::query("SELECT id,name,host_user_id,rule_preset,bot_difficulty,status::text,invite_code,is_public,turn_seconds,(SELECT count(*) FROM lobby_spectators WHERE lobby_id=$1),ranked,rematch_mode FROM game_lobbies WHERE id=$1").bind(lobby_id).fetch_optional(&state.db).await?.ok_or_else(|| ApiError::bad_request("Lobby not found"))?;
    let members = sqlx::query("SELECT m.seat,u.id,u.display_name,m.ready,CASE WHEN m.last_seen_at>CURRENT_TIMESTAMP-INTERVAL '20 seconds' THEN 'online' WHEN m.last_seen_at>CURRENT_TIMESTAMP-INTERVAL '90 seconds' THEN 'reconnecting' ELSE 'offline' END FROM lobby_members m JOIN users u ON u.id=m.user_id WHERE m.lobby_id=$1 ORDER BY m.seat").bind(lobby_id).fetch_all(&state.db).await?;
    let mut seats = (0_i16..4)
        .map(|seat| SeatView {
            seat,
            user_id: None,
            name: format!("Royal Bot {}", seat + 1),
            is_bot: true,
            ready: true,
            presence: "bot".to_owned(),
        })
        .collect::<Vec<_>>();
    for member in members {
        let seat: i16 = member.get(0);
        seats[usize::try_from(seat).unwrap_or(0)] = SeatView {
            seat,
            user_id: Some(member.get(1)),
            name: member.get(2),
            is_bot: false,
            ready: member.get(3),
            presence: member.get(4),
        };
    }
    let requests = if row.get::<Uuid, _>(2) == user_id {
        sqlx::query("SELECT r.id,u.id,u.display_name FROM lobby_join_requests r JOIN users u ON u.id=r.user_id WHERE r.lobby_id=$1 AND r.status='pending' ORDER BY r.created_at").bind(lobby_id).fetch_all(&state.db).await?.into_iter().map(|r|JoinRequestView{id:r.get(0),user_id:r.get(1),display_name:r.get(2)}).collect()
    } else {
        Vec::new()
    };
    send_to(
        state,
        user_id,
        ServerMessage::Lobby {
            lobby: LobbyView {
                id: row.get(0),
                name: row.get(1),
                host_user_id: row.get(2),
                rule_preset: row.get(3),
                bot_difficulty: row.get(4),
                status: row.get(5),
                invite_code: row.get(6),
                is_public: row.get(7),
                turn_seconds: row.get(8),
                spectator_count: row.get(9),
                ranked: row.get(10),
                rematch_mode: row.get(11),
                seats,
                requests,
            },
        },
    )
    .await;
    Ok(())
}

pub(super) async fn maintain_lobby_lifecycle(state: &AppState) -> Result<(), ApiError> {
    sqlx::query(
        "WITH replacements AS (
           SELECT l.id,(
             SELECT m.user_id FROM lobby_members m
             WHERE m.lobby_id=l.id AND m.user_id<>l.host_user_id
               AND m.last_seen_at>CURRENT_TIMESTAMP-INTERVAL '90 seconds'
             ORDER BY m.joined_at LIMIT 1
           ) AS successor
           FROM game_lobbies l JOIN lobby_members host
             ON host.lobby_id=l.id AND host.user_id=l.host_user_id
           WHERE l.status='waiting'
             AND host.last_seen_at<CURRENT_TIMESTAMP-INTERVAL '90 seconds'
         )
         UPDATE game_lobbies l SET host_user_id=r.successor,updated_at=CURRENT_TIMESTAMP
         FROM replacements r WHERE l.id=r.id AND r.successor IS NOT NULL",
    )
    .execute(&state.db)
    .await?;
    sqlx::query(
        "UPDATE lobby_members m SET ready=TRUE FROM game_lobbies l
         WHERE l.id=m.lobby_id AND l.host_user_id=m.user_id AND l.status='waiting'",
    )
    .execute(&state.db)
    .await?;
    let removed = sqlx::query(
        "DELETE FROM game_lobbies l WHERE l.status='waiting'
         AND l.updated_at<CURRENT_TIMESTAMP-INTERVAL '2 hours'
         AND NOT EXISTS(
           SELECT 1 FROM lobby_members m WHERE m.lobby_id=l.id
             AND m.last_seen_at>CURRENT_TIMESTAMP-INTERVAL '90 seconds')",
    )
    .execute(&state.db)
    .await?;
    if removed.rows_affected() > 0 {
        tracing::info!(
            count = removed.rows_affected(),
            "closed stale waiting rooms"
        );
        broadcast_lobbies(state).await;
    }
    for statement in [
        "DELETE FROM processed_commands WHERE processed_at<CURRENT_TIMESTAMP-INTERVAL '7 days'",
        "DELETE FROM realtime_outbox WHERE published_at<CURRENT_TIMESTAMP-INTERVAL '7 days'",
        "DELETE FROM lobby_events WHERE created_at<CURRENT_TIMESTAMP-INTERVAL '90 days'",
        "DELETE FROM friend_invites WHERE expires_at<CURRENT_TIMESTAMP-INTERVAL '7 days'",
    ] {
        sqlx::query(statement).execute(&state.db).await?;
    }
    let deleted = sqlx::query(
        "DELETE FROM users u USING account_deletion_requests request
         WHERE request.user_id=u.id AND request.cancelled_at IS NULL
           AND request.execute_after<=CURRENT_TIMESTAMP
           AND NOT EXISTS(
             SELECT 1 FROM lobby_members member JOIN game_lobbies lobby ON lobby.id=member.lobby_id
             WHERE member.user_id=u.id AND lobby.status IN ('waiting','playing')
           )",
    )
    .execute(&state.db)
    .await?;
    if deleted.rows_affected() > 0 {
        tracing::info!(
            count = deleted.rows_affected(),
            "executed account deletion requests"
        );
    }
    Ok(())
}

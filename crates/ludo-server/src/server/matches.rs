//! Authoritative match lifecycle, actions, AI frames, settlement, and supervision.

use super::{
    ActivityView, ApiError, AppState, BOT_MOVE_PAUSE_MS, BOT_ROLL_PAUSE_MS, BotRequest, Controller,
    DiceValue, Duration, GameCommand, GameState, GameStatus, GameViewModel, Ordering, ParallelBot,
    Player, PlayerColor, Postgres, RngExt, Row, SeatView, ServerMessage, TURN_SECONDS, TokenId,
    Transaction, TurnPhase, User, Uuid, broadcast_lobbies, enforce_rate_limit, enqueue_outbox,
    level_for_xp, load_user, maintain_lobby_lifecycle, parse_difficulty, parse_preset, player_id,
    send_hub, send_lobby, send_to,
};

pub(super) struct BotFrame {
    pub(super) model: GameViewModel,
    pub(super) delay_ms: u64,
}

#[allow(clippy::too_many_lines)]
pub(super) async fn start_game(
    state: &AppState,
    user: &User,
    lobby_id: Uuid,
) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await?;
    let lobby = sqlx::query("SELECT rule_preset,bot_difficulty,turn_seconds,ranked FROM game_lobbies WHERE id=$1 AND host_user_id=$2 AND status='waiting' FOR UPDATE")
        .bind(lobby_id).bind(user.id).fetch_optional(&mut *tx).await?.ok_or_else(|| ApiError::bad_request("Only the host can start this game"))?;
    let preset: String = lobby.get(0);
    let difficulty: String = lobby.get(1);
    let turn_seconds = u16::try_from(lobby.get::<i16, _>(2)).unwrap_or(TURN_SECONDS);
    let ranked: bool = lobby.get(3);
    let unready: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM lobby_members
         WHERE lobby_id=$1 AND user_id<>$2 AND NOT ready",
    )
    .bind(lobby_id)
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    if unready > 0 {
        return Err(ApiError::conflict("Every human player must be ready"));
    }
    if ranked {
        let humans: i64 =
            sqlx::query_scalar("SELECT count(*) FROM lobby_members WHERE lobby_id=$1")
                .bind(lobby_id)
                .fetch_one(&mut *tx)
                .await?;
        if humans < 2 {
            return Err(ApiError::conflict(
                "Ranked games require at least two human players",
            ));
        }
    }
    let members = sqlx::query("SELECT m.seat,u.id,u.display_name FROM lobby_members m JOIN users u ON u.id=m.user_id WHERE m.lobby_id=$1 ORDER BY m.seat")
        .bind(lobby_id).fetch_all(&mut *tx).await?;
    let mut names: [Option<(Uuid, String)>; 4] = [None, None, None, None];
    for row in members {
        names[usize::try_from(row.get::<i16, _>(0)).unwrap_or(0)] = Some((row.get(1), row.get(2)));
    }
    let bot = parse_difficulty(&difficulty);
    let players = PlayerColor::ALL
        .into_iter()
        .enumerate()
        .map(|(index, color)| {
            let human = names[index].as_ref();
            Player {
                id: player_id(u8::try_from(index).unwrap_or(0)),
                name: human.map_or_else(
                    || format!("Royal Bot {}", index + 1),
                    |(_, name)| name.clone(),
                ),
                color,
                controller: if human.is_some() {
                    Controller::Human
                } else {
                    Controller::Bot
                },
                bot_difficulty: bot,
            }
        })
        .collect();
    let game = GameState::new(players, parse_preset(&preset).rules())
        .map_err(|_| ApiError::internal("Could not create game"))?;
    let json =
        serde_json::to_value(&game).map_err(|_| ApiError::internal("Could not serialize game"))?;
    let initial_frame = serde_json::to_value(vec![GameViewModel::from(&game)])
        .map_err(|_| ApiError::internal("Could not create replay"))?;
    sqlx::query(
        "UPDATE game_lobbies SET status='playing',game_state=$2,replay_states=$4,
        game_instance_id=$5,turn_deadline=CURRENT_TIMESTAMP+($3::text||' seconds')::interval,
        updated_at=CURRENT_TIMESTAMP WHERE id=$1",
    )
    .bind(lobby_id)
    .bind(json)
    .bind(turn_seconds.to_string())
    .bind(initial_frame)
    .bind(Uuid::new_v4())
    .execute(&mut *tx)
    .await?;
    if state.ably.is_some() {
        for (seat, member) in names.iter().enumerate() {
            let Some((user_id, _)) = member else {
                continue;
            };
            enqueue_outbox(
                &mut tx,
                &format!("ludo:user:{user_id}"),
                "event",
                &ServerMessage::GameStarted {
                    lobby_id,
                    player: player_id(u8::try_from(seat).unwrap_or(0)),
                    model: GameViewModel::from(&game),
                    turn_seconds,
                },
            )
            .await?;
        }
    }
    tx.commit().await?;
    if state.ably.is_none() {
        broadcast_game_started(state, lobby_id, &game, turn_seconds).await?;
    }
    broadcast_presence(state, lobby_id).await?;
    broadcast_lobbies(state).await;
    Ok(())
}

pub(super) async fn sync_game(
    state: &AppState,
    user: &User,
    lobby_id: Uuid,
) -> Result<(), ApiError> {
    let row = sqlx::query("SELECT l.game_state,m.seat,l.turn_seconds FROM game_lobbies l JOIN lobby_members m ON m.lobby_id=l.id WHERE l.id=$1 AND m.user_id=$2 AND l.status='playing'")
        .bind(lobby_id).bind(user.id).fetch_optional(&state.db).await?.ok_or_else(|| ApiError::bad_request("Game not found"))?;
    let game: GameState = serde_json::from_value(row.get(0))
        .map_err(|_| ApiError::internal("Stored game is invalid"))?;
    send_to(
        state,
        user.id,
        ServerMessage::GameStarted {
            lobby_id,
            player: player_id(u8::try_from(row.get::<i16, _>(1)).unwrap_or(0)),
            model: GameViewModel::from(&game),
            turn_seconds: u16::try_from(row.get::<i16, _>(2)).unwrap_or(TURN_SECONDS),
        },
    )
    .await;
    send_feed(state, lobby_id, user.id).await?;
    broadcast_presence(state, lobby_id).await?;
    Ok(())
}

/// Replaces a departing human with an AI immediately, including when it is
/// their current turn. This avoids leaving the table stalled until a deadline.
pub(super) async fn leave_match(
    state: &AppState,
    user: &User,
    lobby_id: Uuid,
) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await?;
    let row = sqlx::query(
        "SELECT l.game_state,m.seat,l.turn_seconds FROM game_lobbies l
         JOIN lobby_members m ON m.lobby_id=l.id
         WHERE l.id=$1 AND m.user_id=$2 AND l.status='playing' FOR UPDATE OF l",
    )
    .bind(lobby_id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::bad_request("Game not found"))?;
    let seat = usize::try_from(row.get::<i16, _>(1)).unwrap_or(4);
    let turn_seconds = u16::try_from(row.get::<i16, _>(2)).unwrap_or(TURN_SECONDS);
    let mut game: GameState = serde_json::from_value(row.get(0))
        .map_err(|_| ApiError::internal("Stored game is invalid"))?;
    game.update_player_control(
        player_id(u8::try_from(seat).unwrap_or(0)),
        format!("Royal Bot {}", seat + 1),
        Controller::Bot,
    )
    .map_err(|_| ApiError::internal("Could not replace player with AI"))?;
    sqlx::query("DELETE FROM lobby_members WHERE lobby_id=$1 AND user_id=$2")
        .bind(lobby_id)
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    let mut frames = run_bots(&mut game);
    let status = if game.status() == GameStatus::Finished {
        "finished"
    } else {
        "playing"
    };
    let replay = serde_json::to_value(
        frames
            .iter()
            .map(|frame| frame.model.clone())
            .collect::<Vec<_>>(),
    )
    .map_err(|_| ApiError::internal("Could not update replay"))?;
    sqlx::query(
        "UPDATE game_lobbies SET game_state=$2,status=$3::lobby_status,
         replay_states=replay_states||$5::jsonb,
         turn_deadline=CASE WHEN $3='playing' THEN CURRENT_TIMESTAMP+($4::text||' seconds')::interval ELSE NULL END,
         updated_at=CURRENT_TIMESTAMP WHERE id=$1",
    )
    .bind(lobby_id)
    .bind(serde_json::to_value(&game).map_err(|_| ApiError::internal("Could not save game"))?)
    .bind(status)
    .bind(turn_seconds.to_string())
    .bind(replay)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    send_to(
        state,
        user.id,
        ServerMessage::MatchEnded {
            lobby_id,
            message: "You left the match. An AI has taken your seat.".to_owned(),
        },
    )
    .await;
    for frame in frames.drain(..) {
        broadcast_model(state, lobby_id, frame.model, turn_seconds).await?;
    }
    broadcast_lobbies(state).await;
    Ok(())
}

/// Allows the host to stop an active unranked table for everyone.
pub(super) async fn end_game(
    state: &AppState,
    user: &User,
    lobby_id: Uuid,
) -> Result<(), ApiError> {
    let stopped = sqlx::query(
        "UPDATE game_lobbies SET status='finished',turn_deadline=NULL,updated_at=CURRENT_TIMESTAMP
         WHERE id=$1 AND host_user_id=$2 AND status='playing' AND NOT ranked",
    )
    .bind(lobby_id)
    .bind(user.id)
    .execute(&state.db)
    .await?;
    if stopped.rows_affected() == 0 {
        return Err(ApiError::bad_request(
            "Only the host can end an active unranked game",
        ));
    }
    let users: Vec<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM lobby_members WHERE lobby_id=$1 UNION SELECT user_id FROM lobby_spectators WHERE lobby_id=$1",
    ).bind(lobby_id).fetch_all(&state.db).await?;
    for user_id in users {
        send_to(
            state,
            user_id,
            ServerMessage::MatchEnded {
                lobby_id,
                message: "The host ended this match.".to_owned(),
            },
        )
        .await;
    }
    broadcast_lobbies(state).await;
    Ok(())
}

pub(super) async fn spectate(
    state: &AppState,
    user: &User,
    lobby_id: Uuid,
) -> Result<(), ApiError> {
    let game_json: serde_json::Value = sqlx::query_scalar(
        "SELECT game_state FROM game_lobbies
         WHERE id=$1 AND status='playing' AND is_public",
    )
    .bind(lobby_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::bad_request("This match is not available to spectate"))?;
    sqlx::query(
        "INSERT INTO lobby_spectators(lobby_id,user_id) VALUES($1,$2)
         ON CONFLICT(lobby_id,user_id)
         DO UPDATE SET last_seen_at=CURRENT_TIMESTAMP",
    )
    .bind(lobby_id)
    .bind(user.id)
    .execute(&state.db)
    .await?;
    let game: GameState = serde_json::from_value(game_json)
        .map_err(|_| ApiError::internal("Stored game is invalid"))?;
    send_to(
        state,
        user.id,
        ServerMessage::SpectatorStarted {
            lobby_id,
            model: GameViewModel::from(&game),
            turn_seconds: TURN_SECONDS,
        },
    )
    .await;
    send_feed(state, lobby_id, user.id).await?;
    broadcast_presence(state, lobby_id).await?;
    Ok(())
}

pub(super) async fn add_activity(
    state: &AppState,
    user: &User,
    lobby_id: Uuid,
    kind: &str,
    raw_message: &str,
) -> Result<(), ApiError> {
    enforce_rate_limit(state, &format!("activity:{}", user.id), 20).await?;
    let allowed = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM lobby_members WHERE lobby_id=$1 AND user_id=$2)
         OR EXISTS(SELECT 1 FROM lobby_spectators WHERE lobby_id=$1 AND user_id=$2)",
    )
    .bind(lobby_id)
    .bind(user.id)
    .fetch_one(&state.db)
    .await?;
    if !allowed {
        return Err(ApiError::unauthorized("Join this match before posting"));
    }
    let message = if kind == "reaction" {
        if !["👍", "👏", "😮", "😂", "🔥", "👑"].contains(&raw_message) {
            return Err(ApiError::bad_request("Unsupported reaction"));
        }
        format!("{} reacted {raw_message}", user.display_name)
    } else {
        let body = raw_message.trim();
        if body.is_empty() || body.chars().count() > 240 {
            return Err(ApiError::bad_request(
                "Chat messages must be 1–240 characters",
            ));
        }
        let normalized = body.to_ascii_lowercase();
        if [
            "http://",
            "https://",
            "<script",
            "discord.gg",
            "fuck",
            "bitch",
            "nigger",
        ]
        .iter()
        .any(|blocked| normalized.contains(blocked))
        {
            return Err(ApiError::bad_request(
                "Links and unsafe content are not allowed",
            ));
        }
        format!("{}: {body}", user.display_name)
    };
    let row = sqlx::query(
        "INSERT INTO lobby_events(lobby_id,actor_user_id,kind,message)
         VALUES($1,$2,$3,$4)
         RETURNING id,kind,message,created_at::text",
    )
    .bind(lobby_id)
    .bind(user.id)
    .bind(kind)
    .bind(message)
    .fetch_one(&state.db)
    .await?;
    broadcast_activity(
        state,
        lobby_id,
        ActivityView {
            id: row.get(0),
            kind: row.get(1),
            message: row.get(2),
            created_at: row.get(3),
        },
    )
    .await?;
    Ok(())
}

pub(super) async fn send_feed(
    state: &AppState,
    lobby_id: Uuid,
    user_id: Uuid,
) -> Result<(), ApiError> {
    let rows = sqlx::query(
        "SELECT id,kind,message,created_at::text FROM lobby_events
         WHERE lobby_id=$1 ORDER BY id DESC LIMIT 40",
    )
    .bind(lobby_id)
    .fetch_all(&state.db)
    .await?;
    let mut events = rows
        .into_iter()
        .map(|row| ActivityView {
            id: row.get(0),
            kind: row.get(1),
            message: row.get(2),
            created_at: row.get(3),
        })
        .collect::<Vec<_>>();
    events.reverse();
    send_to(state, user_id, ServerMessage::Feed { lobby_id, events }).await;
    Ok(())
}

pub(super) async fn broadcast_activity(
    state: &AppState,
    lobby_id: Uuid,
    event: ActivityView,
) -> Result<(), ApiError> {
    let users: Vec<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM lobby_members WHERE lobby_id=$1
         UNION SELECT user_id FROM lobby_spectators WHERE lobby_id=$1",
    )
    .bind(lobby_id)
    .fetch_all(&state.db)
    .await?;
    for user_id in users {
        send_to(
            state,
            user_id,
            ServerMessage::Activity {
                lobby_id,
                event: event.clone(),
            },
        )
        .await;
    }
    Ok(())
}

pub(super) async fn vote_rematch(
    state: &AppState,
    user: &User,
    lobby_id: Uuid,
) -> Result<(), ApiError> {
    let settings = sqlx::query(
        "SELECT rematch_mode,host_user_id FROM game_lobbies WHERE id=$1 AND status='finished'",
    )
    .bind(lobby_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::bad_request("This match is not ready for a rematch"))?;
    let rematch_mode: String = settings.get(0);
    if rematch_mode == "host" && settings.get::<Uuid, _>(1) != user.id {
        return Err(ApiError::bad_request(
            "The host controls rematches at this table",
        ));
    }
    sqlx::query(
        "INSERT INTO rematch_votes(lobby_id,user_id)
         SELECT $1,$2 WHERE EXISTS(
           SELECT 1 FROM lobby_members m JOIN game_lobbies l ON l.id=m.lobby_id
           WHERE m.lobby_id=$1 AND m.user_id=$2 AND l.status='finished')
         ON CONFLICT DO NOTHING",
    )
    .bind(lobby_id)
    .bind(user.id)
    .execute(&state.db)
    .await?;
    let votes: i64 = sqlx::query_scalar("SELECT count(*) FROM rematch_votes WHERE lobby_id=$1")
        .bind(lobby_id)
        .fetch_one(&state.db)
        .await?;
    let needed: i64 = if rematch_mode == "vote" {
        sqlx::query_scalar("SELECT count(*) FROM lobby_members WHERE lobby_id=$1")
            .bind(lobby_id)
            .fetch_one(&state.db)
            .await?
    } else {
        1
    };
    let users: Vec<Uuid> =
        sqlx::query_scalar("SELECT user_id FROM lobby_members WHERE lobby_id=$1")
            .bind(lobby_id)
            .fetch_all(&state.db)
            .await?;
    for user_id in &users {
        send_to(
            state,
            *user_id,
            ServerMessage::RematchUpdate {
                lobby_id,
                votes,
                needed,
            },
        )
        .await;
    }
    if needed > 0 && votes >= needed {
        sqlx::query(
            "UPDATE game_lobbies SET status='waiting',game_state=NULL,turn_deadline=NULL,
             replay_states='[]'::jsonb,updated_at=CURRENT_TIMESTAMP WHERE id=$1 AND status='finished'",
        )
        .bind(lobby_id)
        .execute(&state.db)
        .await?;
        sqlx::query(
            "UPDATE lobby_members m SET ready=(m.user_id=l.host_user_id)
             FROM game_lobbies l WHERE m.lobby_id=$1 AND l.id=m.lobby_id",
        )
        .bind(lobby_id)
        .execute(&state.db)
        .await?;
        sqlx::query("DELETE FROM rematch_votes WHERE lobby_id=$1")
            .bind(lobby_id)
            .execute(&state.db)
            .await?;
        sqlx::query("DELETE FROM lobby_spectators WHERE lobby_id=$1")
            .bind(lobby_id)
            .execute(&state.db)
            .await?;
        send_lobby(state, lobby_id).await?;
        broadcast_lobbies(state).await;
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
pub(super) async fn apply_action(
    state: &AppState,
    user: &User,
    lobby_id: Uuid,
    revision: u64,
    token: Option<TokenId>,
) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await?;
    let row = sqlx::query("SELECT l.game_state,m.seat,l.turn_seconds FROM game_lobbies l JOIN lobby_members m ON m.lobby_id=l.id WHERE l.id=$1 AND m.user_id=$2 AND l.status='playing' FOR UPDATE OF l")
        .bind(lobby_id).bind(user.id).fetch_optional(&mut *tx).await?.ok_or_else(|| ApiError::bad_request("Game not found"))?;
    let mut game: GameState = serde_json::from_value(row.get(0))
        .map_err(|_| ApiError::internal("Stored game is invalid"))?;
    let seat = usize::try_from(row.get::<i16, _>(1)).unwrap_or(4);
    let turn_seconds = u16::try_from(row.get::<i16, _>(2)).unwrap_or(TURN_SECONDS);
    if game.revision() != revision || game.current_player_index() != seat {
        return Err(ApiError::conflict(
            "The game advanced; refreshing the board",
        ));
    }
    let command = token.map_or_else(random_roll, GameCommand::Move);
    game.apply(command)
        .map_err(|error| ApiError::bad_request(&error.to_string()))?;
    let activity_message = token.map_or_else(
        || format!("{} rolled the dice", user.display_name),
        |token| format!("{} moved token {}", user.display_name, token.index() + 1),
    );
    let activity_row = sqlx::query(
        "INSERT INTO lobby_events(lobby_id,actor_user_id,kind,message)
         VALUES($1,$2,'move',$3) RETURNING id,kind,message,created_at::text",
    )
    .bind(lobby_id)
    .bind(user.id)
    .bind(activity_message)
    .fetch_one(&mut *tx)
    .await?;
    let activity = ActivityView {
        id: activity_row.get(0),
        kind: activity_row.get(1),
        message: activity_row.get(2),
        created_at: activity_row.get(3),
    };
    let human_model = GameViewModel::from(&game);
    let bot_frames = run_bots(&mut game);
    let mut replay_frames = vec![human_model.clone()];
    replay_frames.extend(bot_frames.iter().map(|frame| frame.model.clone()));
    let replay_json = serde_json::to_value(&replay_frames)
        .map_err(|_| ApiError::internal("Could not update replay"))?;
    let status = if game.status() == GameStatus::Finished {
        "finished"
    } else {
        "playing"
    };
    sqlx::query("UPDATE game_lobbies SET game_state=$2,status=$3::lobby_status,
        replay_states=replay_states||$5::jsonb,
        turn_deadline=CASE WHEN $3='playing' THEN CURRENT_TIMESTAMP+($4::text||' seconds')::interval ELSE NULL END,
        updated_at=CURRENT_TIMESTAMP WHERE id=$1")
        .bind(lobby_id).bind(serde_json::to_value(&game).map_err(|_| ApiError::internal("Could not save game"))?)
        .bind(status).bind(turn_seconds.to_string()).bind(replay_json).execute(&mut *tx).await?;
    if game.status() == GameStatus::Finished {
        let member_rows =
            sqlx::query("SELECT seat,user_id FROM lobby_members WHERE lobby_id=$1 ORDER BY seat")
                .bind(lobby_id)
                .fetch_all(&mut *tx)
                .await?;
        let mut player_ids = vec![serde_json::Value::Null; 4];
        let mut winner_user_id = None;
        let winner_seat = game.rankings().first().map(|winner| winner.index());
        let mut members = Vec::new();
        for member in member_rows {
            let seat = usize::try_from(member.get::<i16, _>(0)).unwrap_or(4);
            let member_id: Uuid = member.get(1);
            members.push((seat, member_id));
            if seat < player_ids.len() {
                player_ids[seat] = serde_json::Value::String(member_id.to_string());
            }
            if Some(seat) == winner_seat {
                winner_user_id = Some(member_id);
            }
        }
        let match_id = Uuid::new_v4();
        let settings = sqlx::query(
            "SELECT ranked,replay_states,game_instance_id FROM game_lobbies WHERE id=$1",
        )
        .bind(lobby_id)
        .fetch_one(&mut *tx)
        .await?;
        let ranked: bool = settings.get(0);
        let season_id: Option<Uuid> = if ranked {
            sqlx::query_scalar("SELECT id FROM seasons WHERE active AND starts_at<=CURRENT_TIMESTAMP AND ends_at>CURRENT_TIMESTAMP ORDER BY starts_at DESC LIMIT 1")
                .fetch_optional(&mut *tx).await?
        } else {
            None
        };
        let inserted = sqlx::query(
            "INSERT INTO match_results(id,lobby_id,winner_user_id,player_ids,final_state,replay_states,ranked,season_id,game_instance_id)
             VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)
             ON CONFLICT(game_instance_id) DO NOTHING",
        )
        .bind(match_id)
        .bind(lobby_id)
        .bind(winner_user_id)
        .bind(serde_json::Value::Array(player_ids))
        .bind(
            serde_json::to_value(&game)
                .map_err(|_| ApiError::internal("Could not save match result"))?,
        )
        .bind(settings.get::<serde_json::Value, _>(1))
        .bind(ranked)
        .bind(season_id)
        .bind(settings.get::<Uuid, _>(2))
        .execute(&mut *tx).await?;
        if inserted.rows_affected() > 0 {
            for (seat, member_id) in members {
                let placement = game
                    .rankings()
                    .iter()
                    .position(|id| id.index() == seat)
                    .map_or(4_i16, |index| i16::try_from(index + 1).unwrap_or(4));
                let won = placement == 1;
                let xp_earned = if won { 250 } else { 100 };
                let rating_delta = if ranked {
                    if won { 24 } else { -12 }
                } else {
                    0
                };
                settle_player(
                    &mut tx,
                    match_id,
                    member_id,
                    i16::try_from(seat).unwrap_or(0),
                    placement,
                    xp_earned,
                    rating_delta,
                    won,
                    season_id,
                )
                .await?;
            }
        }
    }
    tx.commit().await?;
    broadcast_activity(state, lobby_id, activity).await?;
    broadcast_model(state, lobby_id, human_model, turn_seconds).await?;
    for frame in bot_frames {
        tokio::time::sleep(Duration::from_millis(frame.delay_ms)).await;
        broadcast_model(state, lobby_id, frame.model, turn_seconds).await?;
    }
    if game.status() == GameStatus::Finished {
        *state.leaderboard_cache.write().await = None;
        state
            .metrics
            .matches_completed_total
            .fetch_add(1, Ordering::Relaxed);
        let users: Vec<Uuid> =
            sqlx::query_scalar("SELECT user_id FROM lobby_members WHERE lobby_id=$1")
                .bind(lobby_id)
                .fetch_all(&state.db)
                .await?;
        for user_id in users {
            if let Some(member) = load_user(state, user_id).await? {
                send_hub(state, &member).await?;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub(super) async fn settle_player(
    tx: &mut Transaction<'_, Postgres>,
    match_id: Uuid,
    user_id: Uuid,
    seat: i16,
    placement: i16,
    xp_earned: i32,
    rating_delta: i32,
    won: bool,
    season_id: Option<Uuid>,
) -> Result<(), ApiError> {
    let win_count = i32::from(won);
    let progress = sqlx::query(
        "INSERT INTO player_progress(user_id,xp,matches,wins,current_streak,best_streak)
         VALUES($1,$2,1,$3,$3,$3)
         ON CONFLICT(user_id) DO UPDATE SET
           xp=player_progress.xp+$2,matches=player_progress.matches+1,
           wins=player_progress.wins+$3,
           current_streak=CASE WHEN $3=1 THEN player_progress.current_streak+1 ELSE 0 END,
           best_streak=GREATEST(player_progress.best_streak,
             CASE WHEN $3=1 THEN player_progress.current_streak+1 ELSE player_progress.best_streak END),
           updated_at=CURRENT_TIMESTAMP
         RETURNING xp,matches,wins,current_streak",
    )
    .bind(user_id).bind(i64::from(xp_earned)).bind(win_count)
    .fetch_one(&mut **tx).await?;
    let mut rating = 1000;
    if let Some(season_id) = season_id {
        rating = sqlx::query_scalar(
            "INSERT INTO season_ratings(season_id,user_id,rating,matches,wins)
             VALUES($1,$2,GREATEST(0,1000+$3),1,$4)
             ON CONFLICT(season_id,user_id) DO UPDATE SET
               rating=GREATEST(0,season_ratings.rating+$3),
               matches=season_ratings.matches+1,wins=season_ratings.wins+$4
             RETURNING rating",
        )
        .bind(season_id)
        .bind(user_id)
        .bind(rating_delta)
        .bind(win_count)
        .fetch_one(&mut **tx)
        .await?;
    }
    sqlx::query(
        "INSERT INTO match_participants(match_id,user_id,seat,placement,xp_earned,rating_delta)
         VALUES($1,$2,$3,$4,$5,$6)",
    )
    .bind(match_id)
    .bind(user_id)
    .bind(seat)
    .bind(placement)
    .bind(xp_earned)
    .bind(rating_delta)
    .execute(&mut **tx)
    .await?;
    let daily = [
        ("play_one", 1, 75, true),
        ("play_three", 3, 150, true),
        ("win_one", 1, 125, won),
    ];
    for (key, target, reward, increment) in daily {
        if !increment {
            continue;
        }
        let claimed_now: bool = sqlx::query_scalar(
            "WITH previous AS (
               SELECT claimed FROM daily_progress WHERE user_id=$1 AND challenge_date=CURRENT_DATE AND challenge_key=$2
             ), advanced AS (
               INSERT INTO daily_progress(user_id,challenge_date,challenge_key,progress,claimed)
               VALUES($1,CURRENT_DATE,$2,1,$3<=1)
               ON CONFLICT(user_id,challenge_date,challenge_key) DO UPDATE SET
                 progress=LEAST($3,daily_progress.progress+1),
                 claimed=daily_progress.claimed OR daily_progress.progress+1 >= $3
               RETURNING claimed
             )
             SELECT advanced.claimed AND NOT COALESCE((SELECT claimed FROM previous),FALSE) FROM advanced",
        ).bind(user_id).bind(key).bind(target).fetch_one(&mut **tx).await?;
        if claimed_now {
            sqlx::query("UPDATE player_progress SET xp=xp+$2 WHERE user_id=$1")
                .bind(user_id)
                .bind(i64::from(reward))
                .execute(&mut **tx)
                .await?;
        }
    }
    let xp: i64 = progress.get(0);
    let matches: i32 = progress.get(1);
    let wins: i32 = progress.get(2);
    let streak: i32 = progress.get(3);
    let mut keys = Vec::new();
    if wins >= 1 {
        keys.push("first_win");
    }
    if matches >= 10 {
        keys.push("veteran_10");
    }
    if streak >= 3 {
        keys.push("streak_3");
    }
    if level_for_xp(xp) >= 5 {
        keys.push("level_5");
    }
    if rating >= 1200 {
        keys.push("ranked_1200");
    }
    for key in keys {
        sqlx::query("INSERT INTO player_achievements(user_id,achievement_key) VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(user_id).bind(key).execute(&mut **tx).await?;
    }
    Ok(())
}

pub(super) fn run_bots(game: &mut GameState) -> Vec<BotFrame> {
    let mut frames = Vec::new();
    let mut steps = 0;
    let mut showed_no_move_roll = false;
    while game.status() == GameStatus::Playing
        && game.current().player.controller == Controller::Bot
        && steps < 128
    {
        let (command, delay_ms, rolling_player) = match game.phase() {
            TurnPhase::AwaitingRoll => {
                let value = DiceValue::new(rand::rng().random_range(1..=6))
                    .unwrap_or_else(|| std::process::abort());
                (
                    GameCommand::Roll(value),
                    BOT_ROLL_PAUSE_MS,
                    Some((
                        game.current_player_index(),
                        game.current().player.name.clone(),
                        value.get(),
                    )),
                )
            }
            TurnPhase::AwaitingMove { legal_tokens, .. } => {
                let difficulty = game.current().player.bot_difficulty;
                let decision = ParallelBot::choose(
                    &BotRequest::new(game.clone(), difficulty).with_thinking_time_ms(0),
                );
                // A roll with no legal move has already advanced the turn in
                // the domain. This branch is only reachable for a malformed
                // persisted state, so do not leave the AI turn stalled.
                let Some(token) = decision.token.or_else(|| legal_tokens.first().copied()) else {
                    break;
                };
                (GameCommand::Move(token), BOT_MOVE_PAUSE_MS, None)
            }
        };
        if game.apply(command).is_err() {
            break;
        }
        let mut model = GameViewModel::from(&*game);
        if let Some((rolling_index, rolling_name, dice)) = rolling_player
            && model.dice.is_none()
        {
            // A roll with no legal move advances the domain turn immediately.
            // Keep one presentation frame so remote players still see that roll.
            for item in &mut model.players {
                item.active = false;
            }
            if let Some(player) = model.players.get_mut(rolling_index) {
                player.active = true;
            }
            model.dice = Some(dice);
            model.human_turn = false;
            model.can_roll = false;
            model.status = format!("{rolling_name} rolled {dice} — no legal move");
            showed_no_move_roll = true;
        }
        frames.push(BotFrame { model, delay_ms });
        steps += 1;
    }
    // The no-legal-move roll frame deliberately keeps the die visible for a
    // moment, but it describes the player who just rolled. Follow it with the
    // authoritative next-turn state so clients do not remain stuck on that
    // presentation frame until the turn deadline expires.
    if showed_no_move_roll
        && game.status() == GameStatus::Playing
        && game.current().player.controller != Controller::Bot
    {
        frames.push(BotFrame {
            model: GameViewModel::from(&*game),
            delay_ms: 0,
        });
    }
    frames
}

pub(super) fn random_roll() -> GameCommand {
    GameCommand::Roll(
        DiceValue::new(rand::rng().random_range(1..=6)).unwrap_or_else(|| std::process::abort()),
    )
}

pub(super) async fn broadcast_game_started(
    state: &AppState,
    lobby_id: Uuid,
    game: &GameState,
    turn_seconds: u16,
) -> Result<(), ApiError> {
    let members = sqlx::query("SELECT user_id,seat FROM lobby_members WHERE lobby_id=$1")
        .bind(lobby_id)
        .fetch_all(&state.db)
        .await?;
    for row in members {
        send_to(
            state,
            row.get(0),
            ServerMessage::GameStarted {
                lobby_id,
                player: player_id(u8::try_from(row.get::<i16, _>(1)).unwrap_or(0)),
                model: GameViewModel::from(game),
                turn_seconds,
            },
        )
        .await;
    }
    Ok(())
}

pub(super) async fn broadcast_model(
    state: &AppState,
    lobby_id: Uuid,
    model: GameViewModel,
    turn_seconds: u16,
) -> Result<(), ApiError> {
    let users: Vec<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM lobby_members WHERE lobby_id=$1
         UNION SELECT user_id FROM lobby_spectators WHERE lobby_id=$1",
    )
    .bind(lobby_id)
    .fetch_all(&state.db)
    .await?;
    for id in users {
        send_to(
            state,
            id,
            ServerMessage::State {
                lobby_id,
                model: model.clone(),
                turn_seconds,
            },
        )
        .await;
    }
    broadcast_presence(state, lobby_id).await?;
    Ok(())
}

pub(super) async fn broadcast_presence(state: &AppState, lobby_id: Uuid) -> Result<(), ApiError> {
    let rows = sqlx::query(
        "SELECT m.seat,u.id,u.display_name,m.ready,
                CASE WHEN m.last_seen_at>CURRENT_TIMESTAMP-INTERVAL '20 seconds' THEN 'online'
                     WHEN m.last_seen_at>CURRENT_TIMESTAMP-INTERVAL '90 seconds' THEN 'reconnecting'
                     ELSE 'offline' END
         FROM lobby_members m
         JOIN users u ON u.id=m.user_id WHERE m.lobby_id=$1 ORDER BY m.seat",
    )
    .bind(lobby_id)
    .fetch_all(&state.db)
    .await?;
    let seats = rows
        .into_iter()
        .map(|row| {
            let id: Uuid = row.get(1);
            SeatView {
                seat: row.get(0),
                user_id: Some(id),
                name: row.get(2),
                is_bot: false,
                ready: row.get(3),
                presence: row.get(4),
            }
        })
        .collect::<Vec<_>>();
    let users: Vec<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM lobby_members WHERE lobby_id=$1
         UNION SELECT user_id FROM lobby_spectators WHERE lobby_id=$1",
    )
    .bind(lobby_id)
    .fetch_all(&state.db)
    .await?;
    for user_id in users {
        send_to(
            state,
            user_id,
            ServerMessage::Presence {
                lobby_id,
                seats: seats.clone(),
            },
        )
        .await;
    }
    Ok(())
}

pub(super) async fn run_match_supervisor(state: AppState) {
    let mut lifecycle_tick = 0_u8;
    loop {
        if let Err(error) = advance_expired_turn(&state).await {
            tracing::error!(status=%error.0, message=%error.1, "turn supervisor failed");
        }
        lifecycle_tick = lifecycle_tick.wrapping_add(1);
        if lifecycle_tick.is_multiple_of(15) {
            let waiting: Vec<Uuid> =
                sqlx::query_scalar("SELECT id FROM game_lobbies WHERE status='waiting'")
                    .fetch_all(&state.db)
                    .await
                    .unwrap_or_default();
            for lobby_id in waiting {
                let _ = send_lobby(&state, lobby_id).await;
            }
            if let Err(error) = maintain_lobby_lifecycle(&state).await {
                tracing::error!(status=%error.0, message=%error.1, "lobby lifecycle maintenance failed");
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

#[allow(clippy::too_many_lines)]
pub(super) async fn advance_expired_turn(state: &AppState) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await?;
    let row = sqlx::query(
        "SELECT id,game_state,turn_seconds FROM game_lobbies
         WHERE status='playing' AND turn_deadline<=CURRENT_TIMESTAMP
         ORDER BY turn_deadline
         FOR UPDATE SKIP LOCKED LIMIT 1",
    )
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        tx.rollback().await?;
        return Ok(());
    };
    let lobby_id: Uuid = row.get(0);
    let turn_seconds = u16::try_from(row.get::<i16, _>(2)).unwrap_or(TURN_SECONDS);
    let mut game: GameState = serde_json::from_value(row.get(1))
        .map_err(|_| ApiError::internal("Stored game is invalid"))?;
    let timed_out_player = game.current().player.name.clone();
    let command = match game.phase() {
        TurnPhase::AwaitingRoll => random_roll(),
        TurnPhase::AwaitingMove { legal_tokens, .. } => {
            let difficulty = game.current().player.bot_difficulty;
            let decision = ParallelBot::choose(
                &BotRequest::new(game.clone(), difficulty).with_thinking_time_ms(0),
            );
            let Some(token) = decision.token.or_else(|| legal_tokens.first().copied()) else {
                return Ok(());
            };
            GameCommand::Move(token)
        }
    };
    game.apply(command)
        .map_err(|error| ApiError::bad_request(&error.to_string()))?;
    let timeout_model = GameViewModel::from(&game);
    let bot_frames = run_bots(&mut game);
    let mut replay_frames = vec![timeout_model.clone()];
    replay_frames.extend(bot_frames.iter().map(|frame| frame.model.clone()));
    let activity_row = sqlx::query(
        "INSERT INTO lobby_events(lobby_id,kind,message)
         VALUES($1,'timeout',$2) RETURNING id,kind,message,created_at::text",
    )
    .bind(lobby_id)
    .bind(format!("AI advanced {timed_out_player}'s timed-out turn"))
    .fetch_one(&mut *tx)
    .await?;
    let activity = ActivityView {
        id: activity_row.get(0),
        kind: activity_row.get(1),
        message: activity_row.get(2),
        created_at: activity_row.get(3),
    };
    let status = if game.status() == GameStatus::Finished {
        "finished"
    } else {
        "playing"
    };
    sqlx::query(
        "UPDATE game_lobbies
         SET game_state=$2,status=$3::lobby_status,
             replay_states=replay_states||$5::jsonb,
             turn_deadline=CASE WHEN $3='playing'
               THEN CURRENT_TIMESTAMP+($4::text||' seconds')::interval ELSE NULL END,
             updated_at=CURRENT_TIMESTAMP
         WHERE id=$1",
    )
    .bind(lobby_id)
    .bind(serde_json::to_value(&game).map_err(|_| ApiError::internal("Could not save timed turn"))?)
    .bind(status)
    .bind(turn_seconds.to_string())
    .bind(
        serde_json::to_value(replay_frames)
            .map_err(|_| ApiError::internal("Could not update replay"))?,
    )
    .execute(&mut *tx)
    .await?;
    if game.status() == GameStatus::Finished {
        record_supervised_result(&mut tx, lobby_id, &game).await?;
    }
    tx.commit().await?;
    broadcast_activity(state, lobby_id, activity).await?;
    tracing::info!(%lobby_id, "advanced expired player turn with temporary AI");
    broadcast_model(state, lobby_id, timeout_model, turn_seconds).await?;
    for frame in bot_frames {
        tokio::time::sleep(Duration::from_millis(frame.delay_ms)).await;
        broadcast_model(state, lobby_id, frame.model, turn_seconds).await?;
    }
    if game.status() == GameStatus::Finished {
        *state.leaderboard_cache.write().await = None;
        state
            .metrics
            .matches_completed_total
            .fetch_add(1, Ordering::Relaxed);
        let users: Vec<Uuid> =
            sqlx::query_scalar("SELECT user_id FROM lobby_members WHERE lobby_id=$1")
                .bind(lobby_id)
                .fetch_all(&state.db)
                .await?;
        for user_id in users {
            if let Some(member) = load_user(state, user_id).await? {
                send_hub(state, &member).await?;
            }
        }
    }
    Ok(())
}

pub(super) async fn record_supervised_result(
    tx: &mut Transaction<'_, Postgres>,
    lobby_id: Uuid,
    game: &GameState,
) -> Result<(), ApiError> {
    let rows =
        sqlx::query("SELECT seat,user_id FROM lobby_members WHERE lobby_id=$1 ORDER BY seat")
            .bind(lobby_id)
            .fetch_all(&mut **tx)
            .await?;
    let mut player_ids = vec![serde_json::Value::Null; 4];
    let winner_seat = game.rankings().first().map(|winner| winner.index());
    let mut winner_user_id = None;
    let mut members = Vec::new();
    for row in rows {
        let seat = usize::try_from(row.get::<i16, _>(0)).unwrap_or(4);
        let user_id: Uuid = row.get(1);
        if seat < 4 {
            player_ids[seat] = serde_json::Value::String(user_id.to_string());
        }
        if Some(seat) == winner_seat {
            winner_user_id = Some(user_id);
        }
        members.push((seat, user_id));
    }
    let settings =
        sqlx::query("SELECT ranked,replay_states,game_instance_id FROM game_lobbies WHERE id=$1")
            .bind(lobby_id)
            .fetch_one(&mut **tx)
            .await?;
    let ranked: bool = settings.get(0);
    let season_id = if ranked {
        sqlx::query_scalar("SELECT id FROM seasons WHERE active AND starts_at<=CURRENT_TIMESTAMP AND ends_at>CURRENT_TIMESTAMP ORDER BY starts_at DESC LIMIT 1")
            .fetch_optional(&mut **tx).await?
    } else {
        None
    };
    let match_id = Uuid::new_v4();
    let inserted = sqlx::query("INSERT INTO match_results(id,lobby_id,winner_user_id,player_ids,final_state,replay_states,ranked,season_id,game_instance_id)
        VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9) ON CONFLICT(game_instance_id) DO NOTHING")
        .bind(match_id).bind(lobby_id).bind(winner_user_id)
        .bind(serde_json::Value::Array(player_ids))
        .bind(serde_json::to_value(game).map_err(|_| ApiError::internal("Could not save match result"))?)
        .bind(settings.get::<serde_json::Value,_>(1)).bind(ranked).bind(season_id)
        .bind(settings.get::<Uuid,_>(2))
        .execute(&mut **tx).await?;
    if inserted.rows_affected() == 0 {
        return Ok(());
    }
    for (seat, user_id) in members {
        let placement = game
            .rankings()
            .iter()
            .position(|id| id.index() == seat)
            .map_or(4_i16, |index| i16::try_from(index + 1).unwrap_or(4));
        let won = placement == 1;
        let rating_delta = if ranked {
            if won { 24 } else { -12 }
        } else {
            0
        };
        settle_player(
            tx,
            match_id,
            user_id,
            i16::try_from(seat).unwrap_or(0),
            placement,
            if won { 250 } else { 100 },
            rating_delta,
            won,
            season_id,
        )
        .await?;
    }
    Ok(())
}

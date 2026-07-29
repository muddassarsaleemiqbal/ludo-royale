//! Player profiles, friendships, invitations, progression, and ranked discovery.

use super::{
    ApiError, AppState, ChallengeView, Duration, FriendView, HashMap, HubView, Instant, InviteView,
    LeaderboardCache, LeaderboardView, MatchView, ProfileView, Row, ServerMessage, TURN_SECONDS,
    User, Uuid, accept_joining_player, broadcast_lobbies, create_lobby, send_lobby, send_to,
};

pub(super) fn level_for_xp(xp: i64) -> i64 {
    xp / 500 + 1
}

#[allow(clippy::too_many_lines)]
pub(super) async fn send_hub(state: &AppState, user: &User) -> Result<(), ApiError> {
    sqlx::query("INSERT INTO player_progress(user_id) VALUES($1) ON CONFLICT DO NOTHING")
        .bind(user.id)
        .execute(&state.db)
        .await?;
    let season = sqlx::query(
        "SELECT id,name,ends_at::text FROM seasons
         WHERE active AND starts_at<=CURRENT_TIMESTAMP AND ends_at>CURRENT_TIMESTAMP
         ORDER BY starts_at DESC LIMIT 1",
    )
    .fetch_optional(&state.db)
    .await?;
    let season_id = season.as_ref().map(|row| row.get::<Uuid, _>(0));
    let profile_row = sqlx::query(
        "SELECT p.xp,p.matches,p.wins,p.current_streak,p.best_streak,
                p.selected_dice,p.selected_tokens,COALESCE(r.rating,1000)
         FROM player_progress p
         LEFT JOIN season_ratings r ON r.user_id=p.user_id AND r.season_id=$2
         WHERE p.user_id=$1",
    )
    .bind(user.id)
    .bind(season_id)
    .fetch_one(&state.db)
    .await?;
    let xp: i64 = profile_row.get(0);
    let profile = ProfileView {
        user_id: user.id,
        display_name: user.display_name.clone(),
        xp,
        level: level_for_xp(xp),
        matches: profile_row.get(1),
        wins: profile_row.get(2),
        current_streak: profile_row.get(3),
        best_streak: profile_row.get(4),
        selected_dice: profile_row.get(5),
        selected_tokens: profile_row.get(6),
        rating: profile_row.get(7),
    };
    let friend_rows = sqlx::query(
        "SELECT u.id,u.display_name,p.xp,COALESCE(r.rating,1000),
                CASE WHEN f.status='accepted' THEN 'friend'
                     WHEN f.requester_id=$1 THEN 'outgoing' ELSE 'incoming' END,
                CASE WHEN presence.last_seen_at>CURRENT_TIMESTAMP-INTERVAL '25 seconds'
                     THEN 'online' ELSE 'offline' END
         FROM friendships f
         JOIN users u ON u.id=CASE WHEN f.requester_id=$1 THEN f.addressee_id ELSE f.requester_id END
         LEFT JOIN player_progress p ON p.user_id=u.id
         LEFT JOIN season_ratings r ON r.user_id=u.id AND r.season_id=$2
         LEFT JOIN user_presence presence ON presence.user_id=u.id
         WHERE f.requester_id=$1 OR f.addressee_id=$1
         ORDER BY (f.status='accepted') DESC,u.display_name LIMIT 200",
    )
    .bind(user.id)
    .bind(season_id)
    .fetch_all(&state.db)
    .await?;
    let friends = friend_rows
        .into_iter()
        .map(|row| {
            let id: Uuid = row.get(0);
            FriendView {
                user_id: id,
                display_name: row.get(1),
                level: level_for_xp(row.get::<Option<i64>, _>(2).unwrap_or(0)),
                rating: row.get(3),
                relationship: row.get(4),
                presence: row.get(5),
            }
        })
        .collect();
    let matches = sqlx::query(
        "SELECT m.id,m.completed_at::text,mp.placement,mp.xp_earned,mp.rating_delta,m.ranked,
                ARRAY(
                  SELECT opponent.display_name FROM match_participants other
                  JOIN users opponent ON opponent.id=other.user_id
                  WHERE other.match_id=m.id AND other.user_id<>$1 ORDER BY other.placement
                )
         FROM match_participants mp JOIN match_results m ON m.id=mp.match_id
         WHERE mp.user_id=$1 ORDER BY m.completed_at DESC LIMIT 20",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|row| MatchView {
        id: row.get(0),
        played_at: row.get(1),
        placement: row.get(2),
        xp_earned: row.get(3),
        rating_delta: row.get(4),
        ranked: row.get(5),
        opponents: row.get(6),
    })
    .collect();
    let achievements = sqlx::query_scalar(
        "SELECT achievement_key FROM player_achievements WHERE user_id=$1 ORDER BY unlocked_at",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    let challenge_specs = [
        ("play_one", "Play a match", 1, 75),
        ("win_one", "Win a match", 1, 125),
        ("play_three", "Play three matches", 3, 150),
    ];
    let progress_rows = sqlx::query(
        "SELECT challenge_key,progress,claimed FROM daily_progress
         WHERE user_id=$1 AND challenge_date=CURRENT_DATE",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?;
    let challenge_map = progress_rows
        .into_iter()
        .map(|row| {
            (
                row.get::<String, _>(0),
                (row.get::<i32, _>(1), row.get::<bool, _>(2)),
            )
        })
        .collect::<HashMap<_, _>>();
    let challenges = challenge_specs
        .into_iter()
        .map(|(key, title, target, reward)| {
            let (progress, claimed) = challenge_map.get(key).copied().unwrap_or_default();
            ChallengeView {
                key,
                title,
                progress,
                target,
                reward,
                claimed,
            }
        })
        .collect();
    let leaderboard = if let Some(season_id) = season_id {
        if let Some(cached) = state.leaderboard_cache.read().await.as_ref()
            && cached.season_id == season_id
            && cached.expires_at > Instant::now()
        {
            cached.rows.clone()
        } else {
            let rows = sqlx::query(
                "SELECT row_number() OVER(ORDER BY r.rating DESC,r.wins DESC),u.id,u.display_name,
                    r.rating,r.matches,r.wins
             FROM season_ratings r JOIN users u ON u.id=r.user_id
             WHERE r.season_id=$1 ORDER BY r.rating DESC,r.wins DESC LIMIT 50",
            )
            .bind(season_id)
            .fetch_all(&state.db)
            .await?
            .into_iter()
            .map(|row| LeaderboardView {
                rank: row.get(0),
                user_id: row.get(1),
                display_name: row.get(2),
                rating: row.get(3),
                matches: row.get(4),
                wins: row.get(5),
            })
            .collect::<Vec<_>>();
            *state.leaderboard_cache.write().await = Some(LeaderboardCache {
                season_id,
                expires_at: Instant::now() + Duration::from_secs(30),
                rows: rows.clone(),
            });
            rows
        }
    } else {
        Vec::new()
    };
    let invites = sqlx::query(
        "SELECT i.id,i.lobby_id,l.name,u.display_name FROM friend_invites i
         JOIN game_lobbies l ON l.id=i.lobby_id JOIN users u ON u.id=i.sender_id
         WHERE i.recipient_id=$1 AND i.status='pending' AND i.expires_at>CURRENT_TIMESTAMP
           AND l.status='waiting' ORDER BY i.created_at DESC",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|row| InviteView {
        id: row.get(0),
        lobby_id: row.get(1),
        lobby_name: row.get(2),
        sender_name: row.get(3),
    })
    .collect();
    send_to(
        state,
        user.id,
        ServerMessage::Hub {
            hub: HubView {
                profile,
                friends,
                matches,
                achievements,
                challenges,
                leaderboard,
                season_name: season
                    .as_ref()
                    .map_or("Off season".to_owned(), |row| row.get(1)),
                season_ends_at: season.as_ref().map_or(String::new(), |row| row.get(2)),
                invites,
            },
        },
    )
    .await;
    Ok(())
}

pub(super) async fn search_players(
    state: &AppState,
    user: &User,
    query: &str,
) -> Result<(), ApiError> {
    let query = query.trim();
    if query.chars().count() < 2 {
        send_to(
            state,
            user.id,
            ServerMessage::SearchResults {
                players: Vec::new(),
            },
        )
        .await;
        return Ok(());
    }
    let rows = sqlx::query(
        "SELECT u.id,u.display_name,COALESCE(p.xp,0),COALESCE(r.rating,1000),
                CASE WHEN f.status='accepted' THEN 'friend'
                     WHEN f.requester_id=$1 THEN 'outgoing'
                     WHEN f.addressee_id=$1 THEN 'incoming' ELSE 'none' END,
                CASE WHEN presence.last_seen_at>CURRENT_TIMESTAMP-INTERVAL '25 seconds'
                     THEN 'online' ELSE 'offline' END
         FROM users u LEFT JOIN player_progress p ON p.user_id=u.id
         LEFT JOIN seasons s ON s.active AND s.starts_at<=CURRENT_TIMESTAMP AND s.ends_at>CURRENT_TIMESTAMP
         LEFT JOIN season_ratings r ON r.user_id=u.id AND r.season_id=s.id
         LEFT JOIN user_presence presence ON presence.user_id=u.id
         LEFT JOIN friendships f ON (f.requester_id=$1 AND f.addressee_id=u.id)
                                  OR (f.addressee_id=$1 AND f.requester_id=u.id)
         WHERE u.id<>$1 AND u.display_name ILIKE '%'||$2||'%' ORDER BY u.display_name LIMIT 20",
    )
    .bind(user.id)
    .bind(query)
    .fetch_all(&state.db)
    .await?;
    let players = rows
        .into_iter()
        .map(|row| {
            let id: Uuid = row.get(0);
            FriendView {
                user_id: id,
                display_name: row.get(1),
                level: level_for_xp(row.get(2)),
                rating: row.get(3),
                relationship: row.get(4),
                presence: row.get(5),
            }
        })
        .collect();
    send_to(state, user.id, ServerMessage::SearchResults { players }).await;
    Ok(())
}

pub(super) async fn send_friend_request(
    state: &AppState,
    user: &User,
    other: Uuid,
) -> Result<(), ApiError> {
    let changed = sqlx::query(
        "INSERT INTO friendships(requester_id,addressee_id)
         SELECT $1,$2 WHERE $1<>$2 AND EXISTS(SELECT 1 FROM users WHERE id=$2)
           AND NOT EXISTS(SELECT 1 FROM friendships
             WHERE (requester_id=$1 AND addressee_id=$2) OR (requester_id=$2 AND addressee_id=$1))",
    )
    .bind(user.id)
    .bind(other)
    .execute(&state.db)
    .await?;
    if changed.rows_affected() == 0 {
        return Err(ApiError::conflict("A friendship or request already exists"));
    }
    send_hub(state, user).await?;
    if let Some(other_user) = load_user(state, other).await? {
        send_hub(state, &other_user).await?;
    }
    Ok(())
}

pub(super) async fn respond_friend_request(
    state: &AppState,
    user: &User,
    other: Uuid,
    accept: bool,
) -> Result<(), ApiError> {
    let changed = if accept {
        sqlx::query("UPDATE friendships SET status='accepted',updated_at=CURRENT_TIMESTAMP WHERE requester_id=$1 AND addressee_id=$2 AND status='pending'")
            .bind(other).bind(user.id).execute(&state.db).await?
    } else {
        sqlx::query("DELETE FROM friendships WHERE requester_id=$1 AND addressee_id=$2 AND status='pending'")
            .bind(other).bind(user.id).execute(&state.db).await?
    };
    if changed.rows_affected() == 0 {
        return Err(ApiError::bad_request(
            "Friend request is no longer available",
        ));
    }
    send_hub(state, user).await?;
    if let Some(other_user) = load_user(state, other).await? {
        send_hub(state, &other_user).await?;
    }
    Ok(())
}

pub(super) async fn remove_friend(
    state: &AppState,
    user: &User,
    other: Uuid,
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM friendships WHERE (requester_id=$1 AND addressee_id=$2) OR (requester_id=$2 AND addressee_id=$1)")
        .bind(user.id).bind(other).execute(&state.db).await?;
    send_hub(state, user).await?;
    if let Some(other_user) = load_user(state, other).await? {
        send_hub(state, &other_user).await?;
    }
    Ok(())
}

pub(super) async fn invite_friend(
    state: &AppState,
    user: &User,
    lobby_id: Uuid,
    other: Uuid,
) -> Result<(), ApiError> {
    let changed = sqlx::query(
        "INSERT INTO friend_invites(id,lobby_id,sender_id,recipient_id)
         SELECT $1,$2,$3,$4 WHERE EXISTS(
           SELECT 1 FROM game_lobbies l WHERE l.id=$2 AND l.host_user_id=$3 AND l.status='waiting')
         AND EXISTS(SELECT 1 FROM friendships f WHERE f.status='accepted' AND
           ((f.requester_id=$3 AND f.addressee_id=$4) OR (f.requester_id=$4 AND f.addressee_id=$3)))
         ON CONFLICT(lobby_id,recipient_id) DO UPDATE SET sender_id=$3,status='pending',
           expires_at=CURRENT_TIMESTAMP+INTERVAL '30 minutes',created_at=CURRENT_TIMESTAMP",
    )
    .bind(Uuid::new_v4())
    .bind(lobby_id)
    .bind(user.id)
    .bind(other)
    .execute(&state.db)
    .await?;
    if changed.rows_affected() == 0 {
        return Err(ApiError::bad_request(
            "Only the host can invite an accepted friend",
        ));
    }
    if let Some(other_user) = load_user(state, other).await? {
        send_hub(state, &other_user).await?;
    }
    Ok(())
}

pub(super) async fn respond_friend_invite(
    state: &AppState,
    user: &User,
    invite_id: Uuid,
    accept: bool,
) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await?;
    let row = sqlx::query(
        "SELECT lobby_id,sender_id FROM friend_invites WHERE id=$1 AND recipient_id=$2
         AND status='pending' AND expires_at>CURRENT_TIMESTAMP FOR UPDATE",
    )
    .bind(invite_id)
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::bad_request("Invitation is no longer available"))?;
    let lobby_id: Uuid = row.get(0);
    let sender_id: Uuid = row.get(1);
    if accept {
        accept_joining_player(&mut tx, lobby_id, user.id).await?;
    }
    sqlx::query("UPDATE friend_invites SET status=$2 WHERE id=$1")
        .bind(invite_id)
        .bind(if accept { "accepted" } else { "declined" })
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    send_hub(state, user).await?;
    if let Some(sender) = load_user(state, sender_id).await? {
        send_hub(state, &sender).await?;
    }
    if accept {
        send_lobby(state, lobby_id).await?;
        broadcast_lobbies(state).await;
    }
    Ok(())
}

pub(super) async fn set_cosmetics(
    state: &AppState,
    user: &User,
    dice: &str,
    tokens: &str,
) -> Result<(), ApiError> {
    let xp: i64 = sqlx::query_scalar("SELECT xp FROM player_progress WHERE user_id=$1")
        .bind(user.id)
        .fetch_optional(&state.db)
        .await?
        .unwrap_or(0);
    let level = level_for_xp(xp);
    let dice_level = match dice {
        "ivory" => 1,
        "obsidian" => 3,
        "emerald" => 5,
        "royal" => 8,
        _ => 99,
    };
    let token_level = match tokens {
        "classic" => 1,
        "neon" => 3,
        "marble" => 5,
        "metallic" => 8,
        _ => 99,
    };
    if level < dice_level || level < token_level {
        return Err(ApiError::bad_request(
            "That cosmetic has not been unlocked yet",
        ));
    }
    sqlx::query("UPDATE player_progress SET selected_dice=$2,selected_tokens=$3,updated_at=CURRENT_TIMESTAMP WHERE user_id=$1")
        .bind(user.id).bind(dice).bind(tokens).execute(&state.db).await?;
    send_hub(state, user).await
}

pub(super) async fn send_replay(
    state: &AppState,
    user: &User,
    match_id: Uuid,
) -> Result<(), ApiError> {
    let value: serde_json::Value = sqlx::query_scalar(
        "SELECT m.replay_states FROM match_results m JOIN match_participants p ON p.match_id=m.id
         WHERE m.id=$1 AND p.user_id=$2",
    )
    .bind(match_id)
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::bad_request("Replay not found"))?;
    let frames =
        serde_json::from_value(value).map_err(|_| ApiError::internal("Replay is invalid"))?;
    send_to(state, user.id, ServerMessage::Replay { match_id, frames }).await;
    Ok(())
}

pub(super) async fn ranked_match(state: &AppState, user: &User) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await?;
    let lobby_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT l.id FROM game_lobbies l WHERE l.ranked AND l.status='waiting'
         AND l.host_user_id<>$1 AND (SELECT count(*) FROM lobby_members m WHERE m.lobby_id=l.id)<4
         AND (
           SELECT count(*) FROM match_results recent
           JOIN match_participants mine ON mine.match_id=recent.id AND mine.user_id=$1
           JOIN match_participants theirs ON theirs.match_id=recent.id AND theirs.user_id=l.host_user_id
           WHERE recent.completed_at>CURRENT_TIMESTAMP-INTERVAL '24 hours'
         ) < 3
         ORDER BY l.created_at FOR UPDATE SKIP LOCKED LIMIT 1",
    )
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(lobby_id) = lobby_id {
        accept_joining_player(&mut tx, lobby_id, user.id).await?;
        sqlx::query("UPDATE lobby_members SET ready=TRUE WHERE lobby_id=$1 AND user_id=$2")
            .bind(lobby_id)
            .bind(user.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        send_lobby(state, lobby_id).await?;
        broadcast_lobbies(state).await;
    } else {
        tx.rollback().await?;
        create_lobby(
            state,
            user,
            "Ranked • Founders Season".to_owned(),
            "classic".to_owned(),
            "hard".to_owned(),
            true,
            TURN_SECONDS,
        )
        .await?;
        sqlx::query("UPDATE game_lobbies SET ranked=TRUE,rematch_mode='vote' WHERE host_user_id=$1 AND status='waiting'")
            .bind(user.id).execute(&state.db).await?;
        let active: Uuid = sqlx::query_scalar(
            "SELECT id FROM game_lobbies WHERE host_user_id=$1 AND status='waiting' ORDER BY created_at DESC LIMIT 1",
        ).bind(user.id).fetch_one(&state.db).await?;
        send_lobby(state, active).await?;
    }
    Ok(())
}

pub(super) async fn load_user(state: &AppState, user_id: Uuid) -> Result<Option<User>, ApiError> {
    Ok(sqlx::query_as::<_, (Uuid, String, String)>(
        "SELECT id,email,display_name FROM users WHERE id=$1",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .map(|(id, email, display_name)| User {
        id,
        email,
        display_name,
    }))
}

//! PostgreSQL-backed invariants. CI supplies `LUDO_TEST_DATABASE_URL`.
#![allow(clippy::panic)]

use super::{PgPool, PgPoolOptions, Uuid};

async fn test_pool() -> Option<PgPool> {
    let url = std::env::var("LUDO_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .ok()?;
    sqlx::migrate!().run(&pool).await.ok()?;
    Some(pool)
}

async fn user(pool: &PgPool, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id,email,display_name,password_hash) VALUES($1,$2,$3,'test')")
        .bind(id)
        .bind(format!("{id}@integration.test"))
        .bind(name)
        .execute(pool)
        .await
        .unwrap_or_else(|error| panic!("could not create integration user: {error}"));
    id
}

#[tokio::test]
async fn reverse_friend_requests_are_rejected_by_the_database() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let first = user(&pool, "First").await;
    let second = user(&pool, "Second").await;
    sqlx::query("INSERT INTO friendships(requester_id,addressee_id) VALUES($1,$2)")
        .bind(first)
        .bind(second)
        .execute(&pool)
        .await
        .unwrap_or_else(|error| panic!("first friendship failed: {error}"));
    let reverse = sqlx::query("INSERT INTO friendships(requester_id,addressee_id) VALUES($1,$2)")
        .bind(second)
        .bind(first)
        .execute(&pool)
        .await;
    assert!(reverse.is_err());
}

#[tokio::test]
async fn one_game_instance_can_only_be_settled_once() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let host = user(&pool, "Host").await;
    let lobby = Uuid::new_v4();
    let game_instance = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO game_lobbies(
           id,host_user_id,name,rule_preset,bot_difficulty,invite_code,game_instance_id
         ) VALUES($1,$2,'Integration','classic','medium',$3,$4)",
    )
    .bind(lobby)
    .bind(host)
    .bind(lobby.simple().to_string()[..8].to_uppercase())
    .bind(game_instance)
    .execute(&pool)
    .await
    .unwrap_or_else(|error| panic!("lobby insert failed: {error}"));
    let statement =
        "INSERT INTO match_results(id,lobby_id,winner_user_id,player_ids,final_state,game_instance_id)
         VALUES($1,$2,$3,'[]','{}',$4)";
    sqlx::query(statement)
        .bind(Uuid::new_v4())
        .bind(lobby)
        .bind(host)
        .bind(game_instance)
        .execute(&pool)
        .await
        .unwrap_or_else(|error| panic!("first settlement failed: {error}"));
    let duplicate = sqlx::query(statement)
        .bind(Uuid::new_v4())
        .bind(lobby)
        .bind(host)
        .bind(game_instance)
        .execute(&pool)
        .await;
    assert!(duplicate.is_err());
}

#[tokio::test]
async fn active_match_state_survives_a_new_database_connection() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let host = user(&pool, "Recovery Host").await;
    let lobby = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO game_lobbies(
           id,host_user_id,name,rule_preset,bot_difficulty,invite_code,status,game_state,turn_deadline
         ) VALUES($1,$2,'Recovery','classic','medium',$3,'playing','{}',CURRENT_TIMESTAMP)",
    )
    .bind(lobby)
    .bind(host)
    .bind(lobby.simple().to_string()[..8].to_uppercase())
    .execute(&pool)
    .await
    .unwrap_or_else(|error| panic!("recovery lobby insert failed: {error}"));
    pool.close().await;
    let Some(reconnected) = test_pool().await else {
        panic!("database did not reconnect")
    };
    let status: String = sqlx::query_scalar("SELECT status::text FROM game_lobbies WHERE id=$1")
        .bind(lobby)
        .fetch_one(&reconnected)
        .await
        .unwrap_or_default();
    assert_eq!(status, "playing");
}

#[tokio::test]
async fn shared_presence_is_upserted_once_per_player() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let player = user(&pool, "Presence Player").await;
    let statement = "
        INSERT INTO user_presence(user_id,last_seen_at)
        VALUES($1,CURRENT_TIMESTAMP)
        ON CONFLICT(user_id) DO UPDATE SET last_seen_at=CURRENT_TIMESTAMP";
    sqlx::query(statement)
        .bind(player)
        .execute(&pool)
        .await
        .unwrap_or_else(|error| panic!("initial presence failed: {error}"));
    sqlx::query(statement)
        .bind(player)
        .execute(&pool)
        .await
        .unwrap_or_else(|error| panic!("presence update failed: {error}"));
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_presence WHERE user_id=$1")
        .bind(player)
        .fetch_one(&pool)
        .await
        .unwrap_or_default();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn account_deletion_requests_are_unique_and_cascade_with_users() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let player = user(&pool, "Privacy Player").await;
    sqlx::query("INSERT INTO account_deletion_requests(user_id) VALUES($1)")
        .bind(player)
        .execute(&pool)
        .await
        .unwrap_or_else(|error| panic!("deletion request failed: {error}"));
    let duplicate = sqlx::query("INSERT INTO account_deletion_requests(user_id) VALUES($1)")
        .bind(player)
        .execute(&pool)
        .await;
    assert!(duplicate.is_err());

    sqlx::query("DELETE FROM users WHERE id=$1")
        .bind(player)
        .execute(&pool)
        .await
        .unwrap_or_else(|error| panic!("user deletion failed: {error}"));
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM account_deletion_requests WHERE user_id=$1")
            .bind(player)
            .fetch_one(&pool)
            .await
            .unwrap_or_default();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn progression_constraints_reject_invalid_values() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let player = user(&pool, "Progress Player").await;
    let negative_xp = sqlx::query(
        "INSERT INTO player_progress(user_id,xp) VALUES($1,-1)
         ON CONFLICT(user_id) DO UPDATE SET xp=-1",
    )
    .bind(player)
    .execute(&pool)
    .await;
    assert!(negative_xp.is_err());

    let invalid_cosmetic = sqlx::query(
        "INSERT INTO player_progress(user_id,selected_dice) VALUES($1,'pay-to-win')
         ON CONFLICT(user_id) DO UPDATE SET selected_dice='pay-to-win'",
    )
    .bind(player)
    .execute(&pool)
    .await;
    assert!(invalid_cosmetic.is_err());
}

#[tokio::test]
async fn production_hot_path_indexes_exist_after_migration() {
    let Some(pool) = test_pool().await else {
        return;
    };
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT indexname FROM pg_indexes
         WHERE schemaname=current_schema()
           AND indexname = ANY($1)",
    )
    .bind(vec![
        "match_results_completed_idx",
        "friendships_canonical_unique",
        "season_ratings_leaderboard_idx",
        "user_presence_seen_idx",
    ])
    .fetch_all(&pool)
    .await
    .unwrap_or_default();
    assert_eq!(names.len(), 4);
}

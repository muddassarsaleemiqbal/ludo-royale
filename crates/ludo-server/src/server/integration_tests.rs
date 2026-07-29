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

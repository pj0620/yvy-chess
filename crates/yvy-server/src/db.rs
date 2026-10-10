use std::{collections::BTreeMap, time::Duration};

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{postgres::PgPoolOptions, types::Json, PgPool};
use uuid::Uuid;

/// Connects to Postgres and applies pending migrations.
pub async fn connect(database_url: &str) -> anyhow::Result<PgPool> {
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(10))
        .connect(database_url)
        .await
        .with_context(|| format!("connecting to Postgres at {}", redact(database_url)))?;
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("running migrations")?;
    Ok(pool)
}

fn redact(url: &str) -> String {
    match url.find('@') {
        Some(at) => format!("postgres://…@{}", &url[at + 1..]),
        None => url.to_string(),
    }
}

/// chess.com usernames are case-insensitive; we store them lowercased.
pub fn normalize_username(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PlayerRow {
    pub username: String,
    pub player_id: Option<i64>,
    pub joined: Option<DateTime<Utc>>,
    pub last_import_at: Option<DateTime<Utc>>,
    pub ratings: Json<Value>,
}

pub async fn get_player(pool: &PgPool, username: &str) -> sqlx::Result<Option<PlayerRow>> {
    sqlx::query_as::<_, PlayerRow>(
        "SELECT username, player_id, joined, last_import_at, ratings FROM players WHERE username = $1",
    )
    .bind(username)
    .fetch_optional(pool)
    .await
}

pub async fn upsert_player(
    pool: &PgPool,
    username: &str,
    player_id: Option<i64>,
    joined: Option<DateTime<Utc>>,
    ratings: Value,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO players (username, player_id, joined, ratings) VALUES ($1, $2, $3, $4)
         ON CONFLICT (username) DO UPDATE SET
           player_id = COALESCE(EXCLUDED.player_id, players.player_id),
           joined = COALESCE(EXCLUDED.joined, players.joined),
           ratings = CASE WHEN EXCLUDED.ratings = '{}'::jsonb THEN players.ratings ELSE EXCLUDED.ratings END",
    )
    .bind(username)
    .bind(player_id)
    .bind(joined)
    .bind(Json(ratings))
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_last_import(pool: &PgPool, username: &str) -> sqlx::Result<()> {
    sqlx::query("UPDATE players SET last_import_at = now() WHERE username = $1")
        .bind(username)
        .execute(pool)
        .await?;
    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ArchiveRow {
    pub year: i32,
    pub month: i32,
    pub etag: Option<String>,
    pub complete: bool,
}

pub async fn archive_rows(pool: &PgPool, username: &str) -> sqlx::Result<Vec<ArchiveRow>> {
    sqlx::query_as::<_, ArchiveRow>(
        "SELECT year, month, etag, complete FROM archives WHERE username = $1",
    )
    .bind(username)
    .fetch_all(pool)
    .await
}

pub async fn upsert_archive(
    pool: &PgPool,
    username: &str,
    year: i32,
    month: i32,
    etag: Option<&str>,
    game_count: i32,
    complete: bool,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO archives (username, year, month, etag, fetched_at, game_count, complete)
         VALUES ($1, $2, $3, $4, now(), $5, $6)
         ON CONFLICT (username, year, month) DO UPDATE SET
           etag = EXCLUDED.etag, fetched_at = now(), game_count = EXCLUDED.game_count, complete = EXCLUDED.complete",
    )
    .bind(username)
    .bind(year)
    .bind(month)
    .bind(etag)
    .bind(game_count)
    .bind(complete)
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct GameCounts {
    pub total: i64,
    pub white: i64,
    pub black: i64,
    pub positions: i64,
    pub by_time_class: BTreeMap<String, i64>,
    pub first_game: Option<DateTime<Utc>>,
    pub last_game: Option<DateTime<Utc>>,
}

pub async fn game_counts(pool: &PgPool, username: &str) -> sqlx::Result<GameCounts> {
    let (total, white, black, first_game, last_game): (i64, i64, i64, Option<DateTime<Utc>>, Option<DateTime<Utc>>) =
        sqlx::query_as(
            "SELECT COUNT(*), COUNT(*) FILTER (WHERE color = 'white'), COUNT(*) FILTER (WHERE color = 'black'),
                    MIN(end_time), MAX(end_time)
             FROM games WHERE username = $1",
        )
        .bind(username)
        .fetch_one(pool)
        .await?;
    let positions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM position_moves WHERE username = $1")
        .bind(username)
        .fetch_one(pool)
        .await?;
    let by: Vec<(String, i64)> = sqlx::query_as(
        "SELECT time_class, COUNT(*) FROM games WHERE username = $1 GROUP BY time_class ORDER BY time_class",
    )
    .bind(username)
    .fetch_all(pool)
    .await?;
    Ok(GameCounts {
        total,
        white,
        black,
        positions,
        by_time_class: by.into_iter().collect(),
        first_game,
        last_game,
    })
}

/// One game played in the app, as listed on the player page.
#[derive(Debug, Clone, serde::Serialize, sqlx::FromRow)]
pub struct SessionSummary {
    pub id: Uuid,
    pub user_color: String,
    pub engine_elo: i32,
    pub mode: String,
    pub steer_cp: i32,
    pub remember: bool,
    pub gap_mode: String,
    pub current_color: String,
    pub status: String,
    pub result: Option<String>,
    pub plies: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn list_sessions(pool: &PgPool, username: &str, limit: i64) -> sqlx::Result<Vec<SessionSummary>> {
    sqlx::query_as::<_, SessionSummary>(
        "SELECT s.id, s.user_color, s.engine_elo, s.mode, s.steer_cp, s.remember, s.gap_mode, s.current_color, s.status, s.result,
                (SELECT COUNT(*) FROM session_moves m WHERE m.session_id = s.id) AS plies,
                s.created_at, s.updated_at
         FROM sessions s
         WHERE s.username = $1
         ORDER BY s.updated_at DESC
         LIMIT $2",
    )
    .bind(username)
    .bind(limit)
    .fetch_all(pool)
    .await
}

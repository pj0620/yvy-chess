//! Downloads a player's archives from chess.com and indexes every decision they made.

use std::{collections::HashMap, collections::HashSet, sync::Arc};

use anyhow::{anyhow, Context};
use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::sync::watch;
use yvy_core::{extract_player_moves, Color};

use crate::{
    chesscom::{self, ArchiveFetch, Client, Game},
    db,
    state::AppState,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Running,
    Done,
    Failed,
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            JobStatus::Running => "running",
            JobStatus::Done => "done",
            JobStatus::Failed => "failed",
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub username: String,
    pub status: JobStatus,
    pub phase: String,
    pub archives_total: u32,
    pub archives_done: u32,
    pub games_seen: u64,
    pub games_new: u64,
    pub games_skipped: u64,
    pub positions_new: u64,
    pub errors: Vec<String>,
    pub message: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl Progress {
    pub fn new(username: &str) -> Self {
        Progress {
            username: username.to_string(),
            status: JobStatus::Running,
            phase: "starting".to_string(),
            archives_total: 0,
            archives_done: 0,
            games_seen: 0,
            games_new: 0,
            games_skipped: 0,
            positions_new: 0,
            errors: Vec::new(),
            message: None,
            started_at: Utc::now(),
            finished_at: None,
        }
    }

    fn push_error(&mut self, e: String) {
        if self.errors.len() < 50 {
            self.errors.push(e);
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ImportOptions {
    /// Only import the most recent N monthly archives.
    pub months: Option<u32>,
}

#[derive(Debug, Default)]
pub struct IndexStats {
    pub seen: u64,
    pub new: u64,
    pub skipped: u64,
    pub positions: u64,
    pub errors: Vec<String>,
}

/// Starts an import in the background and registers it in the app state. If an import for
/// the user is already running, its receiver is returned instead.
pub async fn spawn(state: AppState, username: String, opts: ImportOptions) -> watch::Receiver<Progress> {
    let mut imports = state.imports.lock().await;
    if let Some(rx) = imports.get(&username) {
        if rx.borrow().status == JobStatus::Running {
            return rx.clone();
        }
    }
    let (tx, rx) = watch::channel(Progress::new(&username));
    imports.insert(username.clone(), rx.clone());
    let pool = state.pool.clone();
    let client = state.chesscom.clone();
    tokio::spawn(async move {
        if let Err(e) = run_import(pool, client, username.clone(), opts, tx).await {
            tracing::error!(username, error = %e, "import failed");
        }
    });
    rx
}

/// Runs a full import, publishing progress through `tx`. Errors are reflected in the final
/// progress value as well as returned.
pub async fn run_import(
    pool: PgPool,
    client: Arc<Client>,
    username: String,
    opts: ImportOptions,
    tx: watch::Sender<Progress>,
) -> anyhow::Result<()> {
    let mut progress = Progress::new(&username);
    let result = run_inner(&pool, &client, &username, &opts, &tx, &mut progress).await;
    progress.finished_at = Some(Utc::now());
    match &result {
        Ok(()) => {
            progress.status = JobStatus::Done;
            progress.phase = "done".to_string();
        }
        Err(e) => {
            progress.status = JobStatus::Failed;
            progress.message = Some(e.to_string());
        }
    }
    tx.send_replace(progress);
    result
}

async fn run_inner(
    pool: &PgPool,
    client: &Client,
    username: &str,
    opts: &ImportOptions,
    tx: &watch::Sender<Progress>,
    progress: &mut Progress,
) -> anyhow::Result<()> {
    progress.phase = "player".to_string();
    tx.send_replace(progress.clone());
    let player = client
        .player(username)
        .await
        .context("looking up the player")?
        .ok_or_else(|| anyhow!("chess.com user '{username}' was not found"))?;
    let stats = client.stats(username).await.unwrap_or(serde_json::Value::Null);
    let joined = player.joined.and_then(|s| DateTime::from_timestamp(s, 0));
    db::upsert_player(pool, username, player.player_id, joined, chesscom::ratings_from_stats(&stats)).await?;

    progress.phase = "archives".to_string();
    tx.send_replace(progress.clone());
    let mut archives: Vec<(i32, i32, String)> = client
        .archives(username)
        .await
        .context("listing archives")?
        .into_iter()
        .filter_map(|url| chesscom::parse_archive_month(&url).map(|(y, m)| (y, m, url)))
        .collect();
    archives.sort();
    if let Some(n) = opts.months {
        let keep = (n as usize).min(archives.len());
        archives = archives.split_off(archives.len() - keep);
    }
    progress.archives_total = archives.len() as u32;
    progress.phase = "games".to_string();
    tx.send_replace(progress.clone());

    let known: HashMap<(i32, i32), db::ArchiveRow> = db::archive_rows(pool, username)
        .await?
        .into_iter()
        .map(|r| ((r.year, r.month), r))
        .collect();
    let now = Utc::now();
    let current = (now.year(), now.month() as i32);

    for (year, month, url) in archives {
        let is_past = (year, month) < current;
        let prior = known.get(&(year, month));
        // A complete month without an ETag can never change: skip it without a request.
        if prior.map(|p| p.complete && p.etag.is_none()).unwrap_or(false) {
            progress.archives_done += 1;
            tx.send_replace(progress.clone());
            continue;
        }
        let etag = prior.filter(|p| p.complete).and_then(|p| p.etag.clone());
        match client.archive(&url, etag.as_deref()).await {
            Ok(ArchiveFetch::NotModified) => {}
            Ok(ArchiveFetch::Fetched { games, etag }) => {
                let stats = index_games(pool, username, &games).await?;
                progress.games_seen += stats.seen;
                progress.games_new += stats.new;
                progress.games_skipped += stats.skipped;
                progress.positions_new += stats.positions;
                for e in stats.errors {
                    progress.push_error(e);
                }
                db::upsert_archive(pool, username, year, month, etag.as_deref(), games.len() as i32, is_past).await?;
            }
            Err(e) => progress.push_error(format!("{year}-{month:02}: {e}")),
        }
        progress.archives_done += 1;
        tx.send_replace(progress.clone());
    }

    db::set_last_import(pool, username).await?;
    Ok(())
}

/// Stores games and indexes the player's decisions. Games already in the database, games
/// with non-standard rules and games whose PGN cannot be replayed are skipped.
pub async fn index_games(pool: &PgPool, username: &str, games: &[Game]) -> anyhow::Result<IndexStats> {
    let mut stats = IndexStats {
        seen: games.len() as u64,
        ..Default::default()
    };
    let ids: Vec<String> = games.iter().map(|g| g.url.clone()).collect();
    let existing: HashSet<String> = sqlx::query_scalar("SELECT id FROM games WHERE id = ANY($1)")
        .bind(&ids)
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect();

    let mut pm_game: Vec<String> = Vec::new();
    let mut pm_ply: Vec<i32> = Vec::new();
    let mut pm_epd: Vec<String> = Vec::new();
    let mut pm_uci: Vec<String> = Vec::new();
    let mut pm_san: Vec<String> = Vec::new();
    let mut pm_setup: Vec<String> = Vec::new();

    let mut tx = pool.begin().await?;
    for g in games {
        if existing.contains(&g.url) {
            continue;
        }
        if g.rules != "chess" {
            stats.skipped += 1;
            continue;
        }
        let Some(pgn) = g.pgn.as_deref() else {
            stats.skipped += 1;
            continue;
        };
        let color = if g.white.username.eq_ignore_ascii_case(username) {
            Color::White
        } else if g.black.username.eq_ignore_ascii_case(username) {
            Color::Black
        } else {
            stats.skipped += 1;
            continue;
        };
        let extracted = match extract_player_moves(pgn, color) {
            Ok(x) => x,
            Err(e) => {
                stats.skipped += 1;
                if stats.errors.len() < 50 {
                    stats.errors.push(format!("{}: {e}", g.url));
                }
                continue;
            }
        };
        let (me, opponent) = match color {
            Color::White => (&g.white, &g.black),
            Color::Black => (&g.black, &g.white),
        };
        sqlx::query(
            "INSERT INTO games (id, username, color, time_class, time_control, rules, rated, end_time, result,
                                opponent, player_rating, opponent_rating, eco, ply_count, pgn)
             VALUES ($1, $2, $3, $4, $5, $6, $7, to_timestamp($8::bigint), $9, $10, $11, $12, $13, $14, $15)
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(&g.url)
        .bind(username)
        .bind(match color {
            Color::White => "white",
            Color::Black => "black",
        })
        .bind(&g.time_class)
        .bind(&g.time_control)
        .bind(&g.rules)
        .bind(g.rated)
        .bind(g.end_time)
        .bind(&me.result)
        .bind(&opponent.username)
        .bind(me.rating)
        .bind(opponent.rating)
        .bind(&g.eco)
        .bind(extracted.ply_count as i32)
        .bind(pgn)
        .execute(&mut *tx)
        .await?;
        for m in &extracted.moves {
            pm_game.push(g.url.clone());
            pm_ply.push(m.ply as i32);
            pm_epd.push(m.epd.clone());
            pm_uci.push(m.uci.clone());
            pm_san.push(m.san.clone());
            pm_setup.push(m.setup.clone());
        }
        stats.new += 1;
        stats.positions += extracted.moves.len() as u64;
    }
    if !pm_game.is_empty() {
        sqlx::query(POSITION_UPSERT)
            .bind(&pm_game)
            .bind(&pm_ply)
            .bind(username)
            .bind(&pm_epd)
            .bind(&pm_uci)
            .bind(&pm_san)
            .bind(&pm_setup)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(stats)
}

pub(crate) const POSITION_UPSERT: &str = "INSERT INTO position_moves (game_id, ply, username, epd, uci, san, setup)
     SELECT u.game_id, u.ply, $3, u.epd, u.uci, u.san, u.setup
     FROM UNNEST($1::text[], $2::int[], $4::text[], $5::text[], $6::text[], $7::text[]) AS u(game_id, ply, epd, uci, san, setup)
     ON CONFLICT (game_id, ply) DO UPDATE SET
       epd = EXCLUDED.epd, uci = EXCLUDED.uci, san = EXCLUDED.san, setup = EXCLUDED.setup";

/// Rebuilds a player's position index from the stored PGNs, so an index that gained columns
/// (such as the own-piece setup) does not require downloading the games again.
pub async fn reindex(pool: &PgPool, username: &str) -> anyhow::Result<u64> {
    let games: Vec<(String, String, String)> =
        sqlx::query_as("SELECT id, color, pgn FROM games WHERE username = $1 ORDER BY end_time")
            .bind(username)
            .fetch_all(pool)
            .await?;
    let mut rows = 0u64;
    for chunk in games.chunks(200) {
        let mut g = Vec::new();
        let mut ply = Vec::new();
        let mut epd = Vec::new();
        let mut uci = Vec::new();
        let mut san = Vec::new();
        let mut setup = Vec::new();
        for (id, color, pgn) in chunk {
            let color = if color == "white" { Color::White } else { Color::Black };
            let Ok(extracted) = extract_player_moves(pgn, color) else { continue };
            for m in extracted.moves {
                g.push(id.clone());
                ply.push(m.ply as i32);
                epd.push(m.epd);
                uci.push(m.uci);
                san.push(m.san);
                setup.push(m.setup);
            }
        }
        if g.is_empty() {
            continue;
        }
        rows += g.len() as u64;
        sqlx::query(POSITION_UPSERT)
            .bind(&g)
            .bind(&ply)
            .bind(username)
            .bind(&epd)
            .bind(&uci)
            .bind(&san)
            .bind(&setup)
            .execute(pool)
            .await?;
    }
    Ok(rows)
}

/// Players whose index predates the `setup` column.
pub async fn players_needing_reindex(pool: &PgPool) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar("SELECT DISTINCT username FROM position_moves WHERE setup IS NULL")
        .fetch_all(pool)
        .await
}

/// Rebuilds stale indexes in the background at startup. Steering simply has less to work
/// with until each player's rebuild finishes.
pub fn spawn_backfill(pool: PgPool) {
    tokio::spawn(async move {
        let users = match players_needing_reindex(&pool).await {
            Ok(users) => users,
            Err(e) => {
                tracing::error!(error = %e, "could not check which players need a reindex");
                return;
            }
        };
        for username in users {
            tracing::info!(username, "rebuilding position index for setup-based steering");
            match reindex(&pool, &username).await {
                Ok(rows) => tracing::info!(username, rows, "position index rebuilt"),
                Err(e) => tracing::error!(username, error = %e, "reindex failed"),
            }
        }
    });
}

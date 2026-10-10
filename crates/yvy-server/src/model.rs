//! The "you" model: what did this player do from a given position?

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use shakmaty::{fen::Fen, CastlingMode, Chess, Position};
use sqlx::PgPool;
use yvy_core::position_key;

/// Restricts which games count. Applied at query time, so no re-indexing is ever needed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Filters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_classes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_from: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_to: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rated: Option<bool>,
}

impl Filters {
    pub fn normalized(mut self) -> Self {
        self.time_classes = self.time_classes.and_then(|v| {
            let v: Vec<String> = v
                .into_iter()
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect();
            if v.is_empty() {
                None
            } else {
                Some(v)
            }
        });
        self
    }
}

/// One move the player made from a position, with how often and in which games.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct Candidate {
    pub uci: String,
    pub san: String,
    pub count: i64,
    /// Up to ten most recent games in which this move was played (chess.com URLs or app sessions).
    pub game_ids: Vec<String>,
    /// When the move was last played, so a habit from years ago is recognizable as such.
    pub last_played: Option<DateTime<Utc>>,
}

/// Moves the player made from `epd`, most frequent first.
pub async fn candidates(pool: &PgPool, username: &str, epd: &str, filters: &Filters) -> sqlx::Result<Vec<Candidate>> {
    sqlx::query_as::<_, Candidate>(
        "SELECT pm.uci, MIN(pm.san) AS san, COUNT(*) AS count,
                (array_agg(pm.game_id ORDER BY g.end_time DESC))[1:10] AS game_ids,
                MAX(g.end_time) AS last_played
         FROM position_moves pm
         JOIN games g ON g.id = pm.game_id
         WHERE pm.username = $1 AND pm.epd = $2
           AND ($3::text[] IS NULL OR g.time_class = ANY($3))
           AND ($4::timestamptz IS NULL OR g.end_time >= $4)
           AND ($5::timestamptz IS NULL OR g.end_time <= $5)
           AND ($6::boolean IS NULL OR g.rated = $6)
         GROUP BY pm.uci
         ORDER BY 3 DESC, 1 ASC",
    )
    .bind(username)
    .bind(epd)
    .bind(&filters.time_classes)
    .bind(filters.date_from)
    .bind(filters.date_to)
    .bind(filters.rated)
    .fetch_all(pool)
    .await
}

pub fn total_games(candidates: &[Candidate]) -> i64 {
    candidates.iter().map(|c| c.count).sum()
}

/// Converts a full FEN into the position key used by the index.
pub fn key_from_fen(fen: &str) -> Result<String, String> {
    let fen: Fen = fen.trim().parse().map_err(|e| format!("invalid FEN: {e}"))?;
    let pos: Chess = fen
        .into_position(CastlingMode::Standard)
        .map_err(|e| format!("illegal position: {e}"))?;
    Ok(position_key(&pos))
}

/// A gap-filling move ranked by how strongly it leads back toward positions the player has
/// faced: `games_ahead` counts the past games passing through any position one reply later.
#[derive(Debug, Clone, Serialize)]
pub struct SteerCandidate {
    pub uci: String,
    pub san: String,
    pub games_ahead: i64,
    /// How many distinct replies of the opponent lead into the player's history.
    pub known_replies: i64,
    /// Past games in which the player's own pieces stood exactly as they would after this move.
    pub setup_games: i64,
}

/// A move for the side to move whose resulting position is in the player's history.
#[derive(Debug, Clone, Serialize)]
pub struct ReturningMove {
    pub uci: String,
    pub san: String,
    pub games: i64,
}

/// Number of past games (under the filters) in which the player faced each of `keys`.
pub async fn games_at(
    pool: &PgPool,
    username: &str,
    keys: &[String],
    filters: &Filters,
) -> sqlx::Result<std::collections::HashMap<String, i64>> {
    if keys.is_empty() {
        return Ok(Default::default());
    }
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT pm.epd, COUNT(*)
         FROM position_moves pm
         JOIN games g ON g.id = pm.game_id
         WHERE pm.username = $1 AND pm.epd = ANY($2)
           AND ($3::text[] IS NULL OR g.time_class = ANY($3))
           AND ($4::timestamptz IS NULL OR g.end_time >= $4)
           AND ($5::timestamptz IS NULL OR g.end_time <= $5)
           AND ($6::boolean IS NULL OR g.rated = $6)
         GROUP BY pm.epd",
    )
    .bind(username)
    .bind(keys)
    .bind(&filters.time_classes)
    .bind(filters.date_from)
    .bind(filters.date_to)
    .bind(filters.rated)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Number of past games (under the filters) in which the player's own pieces stood as in each
/// of the setup `keys`, see [`yvy_core::own_setup_key`].
pub async fn setups_at(
    pool: &PgPool,
    username: &str,
    keys: &[String],
    filters: &Filters,
) -> sqlx::Result<std::collections::HashMap<String, i64>> {
    if keys.is_empty() {
        return Ok(Default::default());
    }
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT pm.setup, COUNT(*)
         FROM position_moves pm
         JOIN games g ON g.id = pm.game_id
         WHERE pm.username = $1 AND pm.setup = ANY($2)
           AND ($3::text[] IS NULL OR g.time_class = ANY($3))
           AND ($4::timestamptz IS NULL OR g.end_time >= $4)
           AND ($5::timestamptz IS NULL OR g.end_time <= $5)
           AND ($6::boolean IS NULL OR g.rated = $6)
         GROUP BY pm.setup",
    )
    .bind(username)
    .bind(keys)
    .bind(&filters.time_classes)
    .bind(filters.date_from)
    .bind(filters.date_to)
    .bind(filters.rated)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

/// For the side to move at `pos`, every legal move that leads back toward the player's games,
/// strongest first. Two signals: `games_ahead` counts past games reached one opponent reply
/// later (move-order transpositions), and `setup_games` counts past games in which the player's
/// own pieces stood exactly as they would after the move (the same development pattern, whatever
/// the opponent did). Exact transpositions rank first.
pub async fn steering_candidates(
    pool: &PgPool,
    username: &str,
    pos: &shakmaty::Chess,
    filters: &Filters,
) -> sqlx::Result<Vec<SteerCandidate>> {
    let firsts = yvy_core::successors(pos);
    let mut reply_keys: Vec<Vec<String>> = Vec::with_capacity(firsts.len());
    let mut all: std::collections::HashSet<String> = std::collections::HashSet::new();
    for f in &firsts {
        let keys: Vec<String> = yvy_core::successors(&f.pos).into_iter().map(|r| r.key).collect();
        all.extend(keys.iter().cloned());
        reply_keys.push(keys);
    }
    let keys: Vec<String> = all.into_iter().collect();
    let counts = games_at(pool, username, &keys, filters).await?;
    let mover = pos.turn();
    let setup_keys: Vec<String> = firsts.iter().map(|f| yvy_core::own_setup_key(&f.pos, mover)).collect();
    let setup_counts = setups_at(pool, username, &setup_keys, filters).await?;
    let mut out: Vec<SteerCandidate> = firsts
        .into_iter()
        .zip(reply_keys)
        .zip(setup_keys)
        .filter_map(|((f, keys), setup)| {
            let mut games = 0i64;
            let mut known = 0i64;
            for k in &keys {
                if let Some(n) = counts.get(k) {
                    games += n;
                    known += 1;
                }
            }
            let setup_games = setup_counts.get(&setup).copied().unwrap_or(0);
            (games > 0 || setup_games > 0).then_some(SteerCandidate {
                uci: f.uci,
                san: f.san,
                games_ahead: games,
                known_replies: known,
                setup_games,
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.games_ahead
            .cmp(&a.games_ahead)
            .then(b.setup_games.cmp(&a.setup_games))
            .then(b.known_replies.cmp(&a.known_replies))
            .then(a.uci.cmp(&b.uci))
    });
    Ok(out)
}

/// Moves for the side to move at `pos` that land directly in the player's history, most
/// played first. Shown to the human so they can steer the game back too.
pub async fn returning_moves(
    pool: &PgPool,
    username: &str,
    pos: &shakmaty::Chess,
    filters: &Filters,
) -> sqlx::Result<Vec<ReturningMove>> {
    let firsts = yvy_core::successors(pos);
    let keys: Vec<String> = firsts.iter().map(|f| f.key.clone()).collect();
    let counts = games_at(pool, username, &keys, filters).await?;
    let mut out: Vec<ReturningMove> = firsts
        .into_iter()
        .filter_map(|f| {
            counts.get(&f.key).map(|n| ReturningMove {
                uci: f.uci,
                san: f.san,
                games: *n,
            })
        })
        .collect();
    out.sort_by(|a, b| b.games.cmp(&a.games).then(a.uci.cmp(&b.uci)));
    Ok(out)
}

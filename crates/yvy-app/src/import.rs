//! Turns one month of chess.com's public API output into indexed decisions, and the compact
//! binary form in which a month is stored in the browser.

use serde::{Deserialize, Serialize};
use yvy_core::{extract_player_moves, Color};

use crate::types::{AppError, AppResult, GameMeta, StoredGame};

/// Bumped whenever the stored month format or the decision encoding changes; older months are
/// then re-imported from chess.com.
pub const BLOB_VERSION: u32 = 1;

#[derive(Deserialize)]
struct Side {
    username: String,
    rating: Option<i32>,
    result: String,
}

#[derive(Deserialize)]
struct ApiGame {
    url: String,
    #[serde(default)]
    pgn: Option<String>,
    time_class: String,
    rules: String,
    #[serde(default)]
    rated: bool,
    end_time: i64,
    #[serde(default)]
    eco: Option<String>,
    white: Side,
    black: Side,
}

#[derive(Deserialize)]
struct ApiMonth {
    games: Vec<ApiGame>,
}

#[derive(Serialize)]
struct BlobRef<'a> {
    version: u32,
    games: &'a [StoredGame],
}

#[derive(Deserialize)]
struct Blob {
    version: u32,
    games: Vec<StoredGame>,
}

/// One month after parsing: every usable game, plus what was skipped and why.
#[derive(Debug, Default)]
pub struct ParsedMonth {
    pub games: Vec<StoredGame>,
    pub seen: u64,
    pub skipped: u64,
    pub errors: Vec<String>,
}

/// Result of importing a month: the blob to store, and what changed in the index.
#[derive(Debug, Default, Serialize)]
pub struct ImportedMonth {
    #[serde(skip)]
    pub blob: Vec<u8>,
    pub seen: u64,
    pub new: u64,
    pub skipped: u64,
    pub positions: u64,
    pub errors: Vec<String>,
}

/// Parses `{"games": [...]}` as returned by `/pub/player/{u}/games/{YYYY}/{MM}`. Games with
/// non-standard rules, without a PGN, without the player, or whose PGN does not replay are
/// skipped; only the last ones are reported as errors.
pub fn parse_month(username: &str, json: &str) -> AppResult<ParsedMonth> {
    let month: ApiMonth =
        serde_json::from_str(json).map_err(|e| AppError::BadRequest(format!("unexpected chess.com response: {e}")))?;
    let mut out = ParsedMonth { seen: month.games.len() as u64, ..Default::default() };
    for g in month.games {
        if g.rules != "chess" {
            out.skipped += 1;
            continue;
        }
        let Some(pgn) = g.pgn.as_deref() else {
            out.skipped += 1;
            continue;
        };
        let color = if g.white.username.eq_ignore_ascii_case(username) {
            Color::White
        } else if g.black.username.eq_ignore_ascii_case(username) {
            Color::Black
        } else {
            out.skipped += 1;
            continue;
        };
        let extracted = match extract_player_moves(pgn, color) {
            Ok(x) => x,
            Err(e) => {
                out.skipped += 1;
                if out.errors.len() < 50 {
                    out.errors.push(format!("{}: {e}", g.url));
                }
                continue;
            }
        };
        let (me, opponent) = match color {
            Color::White => (&g.white, &g.black),
            Color::Black => (&g.black, &g.white),
        };
        out.games.push(StoredGame {
            meta: GameMeta {
                id: g.url.clone(),
                color: if color == Color::White { "white" } else { "black" }.to_string(),
                time_class: g.time_class.to_ascii_lowercase(),
                rated: g.rated,
                end_time: g.end_time * 1000,
                result: me.result.clone(),
                opponent: Some(opponent.username.clone()),
                player_rating: me.rating,
                opponent_rating: opponent.rating,
                eco: g.eco.clone(),
                ply_count: extracted.ply_count,
            },
            decisions: extracted.moves,
        });
    }
    Ok(out)
}

pub fn encode_blob(games: &[StoredGame]) -> Vec<u8> {
    postcard::to_allocvec(&BlobRef { version: BLOB_VERSION, games }).expect("serializing to memory cannot fail")
}

pub fn decode_blob(bytes: &[u8]) -> AppResult<Vec<StoredGame>> {
    let stale = || AppError::Conflict("stored games are from an older version of the app; re-import this account".to_string());
    let blob: Blob = postcard::from_bytes(bytes).map_err(|_| stale())?;
    if blob.version != BLOB_VERSION {
        return Err(stale());
    }
    Ok(blob.games)
}

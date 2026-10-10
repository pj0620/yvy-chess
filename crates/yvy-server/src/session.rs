//! Games played against the model. The server owns the state: every move is validated here,
//! persisted, and answered either from the player's history or by the engine.

use std::collections::HashMap;

use anyhow::anyhow;
use chrono::{DateTime, Utc};
use rand::seq::IndexedRandom;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{types::Json, PgPool};
use uuid::Uuid;
use yvy_core::{choose_index, Color, GameState, Mode, Status};

use crate::{
    db,
    error::{AppError, AppResult},
    model::{self, Candidate, Filters, ReturningMove, SteerCandidate},
    state::AppState,
};

#[derive(Debug, Clone, sqlx::FromRow)]
struct SessionRow {
    id: Uuid,
    username: String,
    user_color: String,
    engine_elo: i32,
    mode: String,
    filters: Json<Filters>,
    steer_cp: i32,
    remember: bool,
    gap_mode: String,
    current_color: String,
    switches: Json<Vec<Switch>>,
    status: String,
    result: Option<String>,
    created_at: DateTime<Utc>,
}

/// The human took over `color` after `ply` moves because the model had no history there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Switch {
    pub ply: i32,
    pub color: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct MoveRow {
    ply: i32,
    uci: String,
    san: String,
    source: String,
    detail: Option<Json<Value>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateSession {
    pub username: String,
    /// "white" or "black": the side the human plays.
    pub user_color: String,
    #[serde(default)]
    pub engine_elo: Option<u32>,
    #[serde(default)]
    pub mode: Option<Mode>,
    #[serde(default)]
    pub filters: Option<Filters>,
    /// Centipawns a gap-filling move may give up to head back toward your games. 0 = off.
    #[serde(default)]
    pub steer_cp: Option<u32>,
    /// Add your moves from this game to your history (default true).
    #[serde(default)]
    pub remember: Option<bool>,
    /// "stockfish" (default) or "switch": what happens when the model has no history.
    #[serde(default)]
    pub gap_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MoveOut {
    pub ply: i32,
    pub uci: String,
    pub san: String,
    /// "player" (the human), "history" (one of your past moves) or "engine" (gap filled).
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

/// Whether the position now on the board exists in the player's history.
#[derive(Debug, Clone, Serialize)]
pub struct Coverage {
    pub in_history: bool,
    pub games: i64,
    pub candidates: Vec<Candidate>,
    /// Moves for the side to move after which the position is back in the player's history.
    pub returning_moves: Vec<ReturningMove>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionState {
    pub id: Uuid,
    pub username: String,
    pub user_color: String,
    pub engine_elo: i32,
    pub mode: Mode,
    pub filters: Filters,
    pub steer_cp: i32,
    pub remember: bool,
    pub gap_mode: String,
    /// The side the human plays right now; equals `user_color` unless a switch happened.
    pub current_color: String,
    pub switches: Vec<Switch>,
    pub status: String,
    pub result: Option<String>,
    pub fen: String,
    pub turn: String,
    pub ply: u32,
    pub legal_moves: Vec<String>,
    pub moves: Vec<MoveOut>,
    pub coverage: Coverage,
    pub created_at: DateTime<Utc>,
}

fn parse_color(s: &str) -> AppResult<Color> {
    match s {
        "white" => Ok(Color::White),
        "black" => Ok(Color::Black),
        other => Err(AppError::BadRequest(format!(
            "user_color must be \"white\" or \"black\", got \"{other}\""
        ))),
    }
}

fn color_str(c: Color) -> &'static str {
    match c {
        Color::White => "white",
        Color::Black => "black",
    }
}

fn mode_str(m: Mode) -> &'static str {
    match m {
        Mode::Sampled => "sampled",
        Mode::MostFrequent => "most_frequent",
    }
}

fn parse_mode(s: &str) -> Mode {
    match s {
        "most_frequent" => Mode::MostFrequent,
        _ => Mode::Sampled,
    }
}

fn status_str(s: Status) -> &'static str {
    match s {
        Status::Playing => "playing",
        Status::Checkmate => "checkmate",
        Status::Stalemate => "stalemate",
        Status::InsufficientMaterial => "insufficient_material",
        Status::ThreefoldRepetition => "threefold_repetition",
        Status::FiftyMoveRule => "fifty_move_rule",
    }
}

/// The player's chess.com rating for the first selected time class, falling back through
/// rapid, blitz, bullet and daily, and finally to 1500.
pub fn default_elo(ratings: &Value, filters: &Filters) -> u32 {
    let preferred: Vec<&str> = match &filters.time_classes {
        Some(v) if !v.is_empty() => v.iter().map(|s| s.as_str()).collect(),
        _ => vec!["rapid", "blitz", "bullet", "daily"],
    };
    for class in preferred {
        if let Some(r) = ratings.get(class).and_then(|v| v.as_u64()) {
            return r as u32;
        }
    }
    1500
}

pub async fn create(state: &AppState, req: CreateSession) -> AppResult<SessionState> {
    let username = db::normalize_username(&req.username);
    let player = db::get_player(&state.pool, &username)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("{username} has not been imported yet")))?;
    let user_color = parse_color(&req.user_color)?;
    let filters = req.filters.unwrap_or_default().normalized();
    let mode = req.mode.unwrap_or_default();
    let engine_elo = req.engine_elo.unwrap_or_else(|| default_elo(&player.ratings.0, &filters)) as i32;
    let steer_cp = req.steer_cp.unwrap_or(100).min(2000) as i32;
    let remember = req.remember.unwrap_or(true);
    let gap_mode = match req.gap_mode.as_deref().unwrap_or("stockfish") {
        "stockfish" => "stockfish",
        "switch" => "switch",
        other => {
            return Err(AppError::BadRequest(format!(
                "gap_mode must be \"stockfish\" or \"switch\", got \"{other}\""
            )))
        }
    };
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO sessions (id, username, user_color, engine_elo, mode, filters, steer_cp, remember, gap_mode, current_color)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $3)",
    )
    .bind(id)
    .bind(&username)
    .bind(color_str(user_color))
    .bind(engine_elo)
    .bind(mode_str(mode))
    .bind(Json(&filters))
    .bind(steer_cp)
    .bind(remember)
    .bind(gap_mode)
    .execute(&state.pool)
    .await?;

    let _guard = state.play_lock.lock().await;
    let (session, _moves, mut game) = load(state, id).await?;
    reply_if_needed(state, &session, &mut game).await?;
    settle(state, &session, &game).await?;
    sync_memory(state, id).await?;
    finish(state, id).await
}

pub async fn get(state: &AppState, id: Uuid) -> AppResult<SessionState> {
    finish(state, id).await
}

pub async fn play(state: &AppState, id: Uuid, uci: &str) -> AppResult<SessionState> {
    let _guard = state.play_lock.lock().await;
    let (session, _moves, mut game) = load(state, id).await?;
    if session.status != "playing" {
        return Err(AppError::Conflict("the game is over".to_string()));
    }
    let user_color = parse_color(&session.current_color)?;
    if game.turn() != user_color {
        return Err(AppError::Conflict("it is not your turn".to_string()));
    }
    let applied = game
        .apply_uci(uci.trim())
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    insert_move(&state.pool, id, game.ply() as i32, &applied.uci, &applied.san, "player", None).await?;
    reply_if_needed(state, &session, &mut game).await?;
    settle(state, &session, &game).await?;
    sync_memory(state, id).await?;
    finish(state, id).await
}

/// Takes back the player's last move together with the reply it received. Because the
/// model is sampled, replaying the same move may get a different answer.
pub async fn undo(state: &AppState, id: Uuid) -> AppResult<SessionState> {
    let _guard = state.play_lock.lock().await;
    let (session, moves, _game) = load(state, id).await?;
    if session.status == "resigned" {
        return Err(AppError::Conflict("cannot undo after resigning".to_string()));
    }
    let mut keep = moves.len();
    if moves.last().map(|m| m.source != "player").unwrap_or(false) {
        keep -= 1;
    }
    if keep > 0 && moves[keep - 1].source == "player" {
        keep -= 1;
    }
    if keep == moves.len() {
        return Err(AppError::Conflict("nothing to undo".to_string()));
    }
    // The side to restore is the one that made the move being taken back.
    let restored = {
        let mut replay = GameState::new();
        for m in &moves[..keep] {
            replay
                .apply_uci(&m.uci)
                .map_err(|e| AppError::Internal(anyhow!("stored moves of session {id} do not replay: {e}")))?;
        }
        color_str(replay.turn()).to_string()
    };
    sqlx::query("DELETE FROM session_moves WHERE session_id = $1 AND ply > $2")
        .bind(id)
        .bind(keep as i32)
        .execute(&state.pool)
        .await?;
    sqlx::query(
        "UPDATE sessions SET status = 'playing', result = NULL, current_color = $2, updated_at = now(),
                switches = (SELECT COALESCE(jsonb_agg(sw), '[]'::jsonb) FROM jsonb_array_elements(switches) sw WHERE (sw->>'ply')::int <= $3)
         WHERE id = $1",
    )
    .bind(id)
    .bind(&restored)
    .bind(keep as i32)
    .execute(&state.pool)
    .await?;
    let (session, _moves, mut game) = load(state, id).await?;
    reply_if_needed(state, &session, &mut game).await?;
    settle(state, &session, &game).await?;
    sync_memory(state, id).await?;
    finish(state, id).await
}

pub async fn resign(state: &AppState, id: Uuid) -> AppResult<SessionState> {
    let _guard = state.play_lock.lock().await;
    let (session, _moves, _game) = load(state, id).await?;
    if session.status != "playing" {
        return Err(AppError::Conflict("the game is already over".to_string()));
    }
    sqlx::query("UPDATE sessions SET status = 'resigned', result = 'loss', updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await?;
    sync_memory(state, id).await?;
    finish(state, id).await
}

async fn load(state: &AppState, id: Uuid) -> AppResult<(SessionRow, Vec<MoveRow>, GameState)> {
    let session = sqlx::query_as::<_, SessionRow>(
        "SELECT id, username, user_color, engine_elo, mode, filters, steer_cp, remember, gap_mode, current_color, switches,
                status, result, created_at
         FROM sessions WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("session not found".to_string()))?;
    let moves = sqlx::query_as::<_, MoveRow>(
        "SELECT ply, uci, san, source, detail FROM session_moves WHERE session_id = $1 ORDER BY ply",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let ucis: Vec<&str> = moves.iter().map(|m| m.uci.as_str()).collect();
    let game = GameState::from_uci_moves(&ucis)
        .map_err(|e| AppError::Internal(anyhow!("stored moves of session {id} do not replay: {e}")))?;
    Ok((session, moves, game))
}

async fn finish(state: &AppState, id: Uuid) -> AppResult<SessionState> {
    let (session, moves, game) = load(state, id).await?;
    let candidates = model::candidates(&state.pool, &session.username, &game.key(), &session.filters.0).await?;
    let games = model::total_games(&candidates);
    let playing = session.status == "playing";
    let user_color = parse_color(&session.current_color)?;
    let returning_moves = if playing && game.turn() == user_color {
        model::returning_moves(&state.pool, &session.username, game.position(), &session.filters.0).await?
    } else {
        Vec::new()
    };
    Ok(SessionState {
        id: session.id,
        username: session.username,
        user_color: session.user_color,
        engine_elo: session.engine_elo,
        mode: parse_mode(&session.mode),
        filters: session.filters.0,
        steer_cp: session.steer_cp,
        remember: session.remember,
        gap_mode: session.gap_mode,
        current_color: session.current_color,
        switches: session.switches.0,
        status: session.status,
        result: session.result,
        fen: game.fen(),
        turn: color_str(game.turn()).to_string(),
        ply: game.ply(),
        legal_moves: if playing { game.legal_moves_uci() } else { Vec::new() },
        moves: moves
            .into_iter()
            .map(|m| MoveOut {
                ply: m.ply,
                uci: m.uci,
                san: m.san,
                source: m.source,
                detail: m.detail.map(|j| j.0),
            })
            .collect(),
        coverage: Coverage {
            in_history: !candidates.is_empty(),
            games,
            candidates,
            returning_moves,
        },
        created_at: session.created_at,
    })
}

async fn insert_move(
    pool: &PgPool,
    id: Uuid,
    ply: i32,
    uci: &str,
    san: &str,
    source: &str,
    detail: Option<Value>,
) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO session_moves (session_id, ply, uci, san, source, detail) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(ply)
    .bind(uci)
    .bind(san)
    .bind(source)
    .bind(detail.map(Json))
    .execute(pool)
    .await?;
    Ok(())
}

/// If it is the model's turn, answers from the player's history or, failing that, the engine.
async fn reply_if_needed(state: &AppState, session: &SessionRow, game: &mut GameState) -> AppResult<()> {
    let user_color = parse_color(&session.current_color)?;
    if game.status().is_over() || game.turn() == user_color {
        return Ok(());
    }
    let candidates = model::candidates(&state.pool, &session.username, &game.key(), &session.filters.0).await?;
    let (uci, source, detail) = if !candidates.is_empty() {
        let counts: Vec<u32> = candidates.iter().map(|c| c.count.max(0) as u32).collect();
        let mode = parse_mode(&session.mode);
        let idx = choose_index(&counts, mode, &mut rand::rng()).expect("candidates are non-empty");
        let chosen = &candidates[idx];
        let detail = json!({
            "games": model::total_games(&candidates),
            "chosen": chosen.uci,
            "chosen_count": chosen.count,
            "mode": mode,
            "candidates": candidates,
        });
        (chosen.uci.clone(), "history", detail)
    } else if session.gap_mode == "switch" {
        // No history for this position: the human takes over the model's side from here.
        let taken = color_str(game.turn());
        let switch = Switch { ply: game.ply() as i32, color: taken.to_string() };
        sqlx::query("UPDATE sessions SET current_color = $2, switches = switches || $3::jsonb, updated_at = now() WHERE id = $1")
            .bind(session.id)
            .bind(taken)
            .bind(Json(vec![switch]))
            .execute(&state.pool)
            .await?;
        return Ok(());
    } else {
        let (uci, detail) = engine_reply(state, session, game).await?;
        (uci, "engine", detail)
    };
    let applied = game
        .apply_uci(&uci)
        .map_err(|e| AppError::Internal(anyhow!("the model produced an illegal move {uci}: {e}")))?;
    insert_move(&state.pool, session.id, game.ply() as i32, &applied.uci, &applied.san, source, Some(detail)).await?;
    Ok(())
}

/// A score that orders mates above any material evaluation, from the side to move's view.
fn score_value(cp: Option<i32>, mate: Option<i32>) -> i64 {
    match (mate, cp) {
        (Some(m), _) if m > 0 => 100_000 - m as i64,
        (Some(m), _) => -100_000 - m as i64,
        (None, Some(c)) => c as i64,
        (None, None) => 0,
    }
}

fn considered(candidates: &[SteerCandidate], scores: &HashMap<String, (Option<i32>, Option<i32>)>) -> Value {
    Value::Array(
        candidates
            .iter()
            .map(|c| {
                let (cp, mate) = scores.get(&c.uci).copied().unwrap_or((None, None));
                json!({
                    "uci": c.uci,
                    "san": c.san,
                    "games_ahead": c.games_ahead,
                    "known_replies": c.known_replies,
                    "setup_games": c.setup_games,
                    "score_cp": cp,
                    "score_mate": mate,
                })
            })
            .collect(),
    )
}

/// Fills a gap in the player's history. Stockfish proposes its best move; if the session allows
/// steering, moves that lead back toward positions the player has faced are scored with the
/// same search budget and the strongest-steering one within the tolerance is played instead.
async fn engine_reply(state: &AppState, session: &SessionRow, game: &GameState) -> AppResult<(String, Value)> {
    let fen = game.fen();
    let elo = session.engine_elo.max(0) as u32;
    let tolerance = session.steer_cp.max(0) as i64;
    let mut steering = if tolerance > 0 {
        model::steering_candidates(&state.pool, &session.username, game.position(), &session.filters.0).await?
    } else {
        Vec::new()
    };
    steering.truncate(8);

    match state.engine.best_move(&fen, elo).await {
        Ok(reply) => {
            let mut detail = json!({
                "engine": "stockfish",
                "elo": reply.elo,
                "skill_level": reply.skill_level,
                "depth": reply.depth,
                "score_cp": reply.score_cp,
                "score_mate": reply.score_mate,
                "movetime_ms": reply.movetime_ms,
            });
            if steering.is_empty() {
                if tolerance > 0 {
                    detail["steer"] = json!({ "tolerance_cp": tolerance, "candidates": 0 });
                }
                return Ok((reply.uci, detail));
            }
            let best_score = score_value(reply.score_cp, reply.score_mate);
            if let Some(c) = steering.iter().find(|c| c.uci == reply.uci) {
                detail["steer"] = json!({
                    "tolerance_cp": tolerance,
                    "chosen_is_best": true,
                    "games_ahead": c.games_ahead,
                    "known_replies": c.known_replies,
                    "setup_games": c.setup_games,
                    "best_uci": reply.uci,
                    "best_score_cp": reply.score_cp,
                    "considered": considered(&steering, &HashMap::new()),
                });
                return Ok((reply.uci, detail));
            }
            let ucis: Vec<String> = steering.iter().map(|c| c.uci.clone()).collect();
            let lines = match state.engine.analyse_moves(&fen, elo, &ucis).await {
                Ok(lines) => lines,
                Err(e) => {
                    tracing::warn!(error = %e, "steering analysis failed; playing the engine's move");
                    Vec::new()
                }
            };
            let scores: HashMap<String, (Option<i32>, Option<i32>)> =
                lines.iter().map(|l| (l.uci.clone(), (l.score_cp, l.score_mate))).collect();
            // Among the candidates that stay within tolerance, prefer exact transpositions, then
            // familiar setups, then the better evaluation.
            let chosen = steering
                .iter()
                .filter_map(|c| {
                    let &(cp, mate) = scores.get(&c.uci)?;
                    let score = score_value(cp, mate);
                    (best_score - score <= tolerance).then_some((c, score))
                })
                .max_by_key(|(c, score)| (c.games_ahead, c.setup_games, *score))
                .map(|(c, _)| c);
            match chosen {
                Some(c) => {
                    let (cp, mate) = scores[&c.uci];
                    detail["score_cp"] = json!(cp);
                    detail["score_mate"] = json!(mate);
                    detail["steer"] = json!({
                        "tolerance_cp": tolerance,
                        "chosen_is_best": false,
                        "games_ahead": c.games_ahead,
                        "known_replies": c.known_replies,
                        "setup_games": c.setup_games,
                        "best_uci": reply.uci,
                        "best_score_cp": reply.score_cp,
                        "best_score_mate": reply.score_mate,
                        "considered": considered(&steering, &scores),
                    });
                    Ok((c.uci.clone(), detail))
                }
                None => {
                    detail["steer"] = json!({
                        "tolerance_cp": tolerance,
                        "rejected": true,
                        "best_uci": reply.uci,
                        "best_score_cp": reply.score_cp,
                        "considered": considered(&steering, &scores),
                    });
                    Ok((reply.uci, detail))
                }
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "engine unavailable, falling back");
            if let Some(c) = steering.first() {
                return Ok((
                    c.uci.clone(),
                    json!({
                        "engine": "random",
                        "reason": e.to_string(),
                        "steer": {
                            "tolerance_cp": tolerance,
                            "chosen_is_best": false,
                            "games_ahead": c.games_ahead,
                            "known_replies": c.known_replies,
                            "setup_games": c.setup_games,
                            "considered": considered(&steering, &HashMap::new()),
                        },
                    }),
                ));
            }
            let legal = game.legal_moves_uci();
            let pick = legal
                .choose(&mut rand::rng())
                .cloned()
                .ok_or_else(|| AppError::Internal(anyhow!("no legal moves in a game that is not over")))?;
            Ok((pick, json!({ "engine": "random", "reason": e.to_string() })))
        }
    }
}

/// Records the outcome once the game is over.
async fn settle(state: &AppState, session: &SessionRow, game: &GameState) -> AppResult<()> {
    let status = game.status();
    if !status.is_over() {
        sqlx::query("UPDATE sessions SET updated_at = now() WHERE id = $1")
            .bind(session.id)
            .execute(&state.pool)
            .await?;
        return Ok(());
    }
    let user_color = parse_color(&session.current_color)?;
    let result = match game.winner() {
        Some(w) if w == user_color => "win",
        Some(_) => "loss",
        None => "draw",
    };
    sqlx::query("UPDATE sessions SET status = $2, result = $3, updated_at = now() WHERE id = $1")
        .bind(session.id)
        .bind(status_str(status))
        .bind(result)
        .execute(&state.pool)
        .await?;
    Ok(())
}

/// One move as the PGN renderer needs it.
struct PgnMove<'a> {
    san: &'a str,
    source: &'a str,
    detail: Option<&'a Value>,
}

fn render_pgn(
    username: &str,
    user_color: &str,
    status: &str,
    result: Option<&str>,
    created_at: DateTime<Utc>,
    moves: &[PgnMove<'_>],
    switches: &[Switch],
) -> String {
    let model_name = format!("{username} (model)");
    let (white, black) = if user_color == "white" {
        (username.to_string(), model_name)
    } else {
        (model_name, username.to_string())
    };
    let result_token = match (status, result) {
        ("playing", _) => "*",
        (_, Some("win")) if user_color == "white" => "1-0",
        (_, Some("win")) => "0-1",
        (_, Some("loss")) if user_color == "white" => "0-1",
        (_, Some("loss")) => "1-0",
        _ => "1/2-1/2",
    };
    let mut out = format!(
        "[Event \"You v You\"]\n[Site \"yvy-chess\"]\n[Date \"{}\"]\n[White \"{white}\"]\n[Black \"{black}\"]\n[Result \"{result_token}\"]\n\n",
        created_at.format("%Y.%m.%d")
    );
    for (i, m) in moves.iter().enumerate() {
        if let Some(sw) = switches.iter().find(|sw| sw.ply as usize == i) {
            out.push_str(&format!("{{you take over as {}}} ", sw.color));
        }
        if i % 2 == 0 {
            out.push_str(&format!("{}. ", i / 2 + 1));
        }
        out.push_str(m.san);
        let note = match m.source {
            "history" => m.detail.and_then(|d| {
                let games = d.get("games")?.as_i64()?;
                let count = d.get("chosen_count")?.as_i64()?;
                Some(format!(" {{you: {count}/{games}}}"))
            }),
            "engine" => m.detail.map(|d| {
                let name = d.get("engine").and_then(|v| v.as_str()).unwrap_or("engine");
                match d.get("elo").and_then(|v| v.as_u64()) {
                    Some(elo) => format!(" {{{name} {elo}}}"),
                    None => format!(" {{{name}}}"),
                }
            }),
            _ => None,
        };
        if let Some(note) = note {
            out.push_str(&note);
        }
        out.push(' ');
    }
    out.push_str(result_token);
    out.push('\n');
    out
}

/// Renders the game as PGN with a comment after every model move saying where it came from.
pub fn to_pgn(s: &SessionState) -> String {
    let moves: Vec<PgnMove<'_>> = s
        .moves
        .iter()
        .map(|m| PgnMove { san: &m.san, source: &m.source, detail: m.detail.as_ref() })
        .collect();
    render_pgn(&s.username, &s.user_color, &s.status, s.result.as_deref(), s.created_at, &moves, &s.switches)
}

/// The game id under which a session is stored in the player's history.
pub fn session_game_id(id: Uuid) -> String {
    format!("yvy:session:{id}")
}

/// chess.com-style result code for the player, so app games sit alongside imported ones.
fn memory_result(status: &str, result: Option<&str>) -> &'static str {
    match (status, result) {
        ("playing", _) => "playing",
        ("resigned", _) => "resigned",
        ("checkmate", Some("win")) => "win",
        ("checkmate", _) => "checkmated",
        ("stalemate", _) => "stalemate",
        ("insufficient_material", _) => "insufficient",
        ("threefold_repetition", _) => "repetition",
        ("fifty_move_rule", _) => "50move",
        (_, Some("win")) => "win",
        _ => "agreed",
    }
}

/// Writes the human's decisions in this session into their history: one game row with time
/// class `yvy`, plus a position row per move they made. Idempotent and re-run after every
/// change, so an undone move disappears from memory again and the game's result stays current.
/// The model's own moves are never recorded; they would turn engine play into "your" moves.
pub async fn sync_memory(state: &AppState, id: Uuid) -> AppResult<()> {
    let (session, moves, _game) = load(state, id).await?;
    if !session.remember {
        return Ok(());
    }
    let game_id = session_game_id(id);
    let mut replay = GameState::new();
    let mut ids = Vec::new();
    let mut plies = Vec::new();
    let mut epds = Vec::new();
    let mut ucis = Vec::new();
    let mut sans = Vec::new();
    let mut setups = Vec::new();
    for m in &moves {
        let before = replay.key();
        let mover = replay.turn();
        let mine = m.source == "player";
        replay
            .apply_uci(&m.uci)
            .map_err(|e| AppError::Internal(anyhow!("stored moves of session {id} do not replay: {e}")))?;
        if mine {
            ids.push(game_id.clone());
            plies.push(m.ply);
            epds.push(before);
            ucis.push(m.uci.clone());
            sans.push(m.san.clone());
            setups.push(yvy_core::own_setup_key(replay.position(), mover));
        }
    }
    let pgn_moves: Vec<PgnMove<'_>> = moves
        .iter()
        .map(|m| PgnMove { san: &m.san, source: &m.source, detail: m.detail.as_ref().map(|j| &j.0) })
        .collect();
    let pgn = render_pgn(
        &session.username,
        &session.user_color,
        &session.status,
        session.result.as_deref(),
        session.created_at,
        &pgn_moves,
        &session.switches.0,
    );

    let mut tx = state.pool.begin().await?;
    sqlx::query(
        "INSERT INTO games (id, username, color, time_class, time_control, rules, rated, end_time, result, opponent, ply_count, pgn)
         VALUES ($1, $2, $3, 'yvy', '-', 'chess', false, now(), $4, $5, $6, $7)
         ON CONFLICT (id) DO UPDATE SET
           end_time = now(), result = EXCLUDED.result, ply_count = EXCLUDED.ply_count, pgn = EXCLUDED.pgn",
    )
    .bind(&game_id)
    .bind(&session.username)
    .bind(&session.user_color)
    .bind(memory_result(&session.status, session.result.as_deref()))
    .bind(format!("{} (model)", session.username))
    .bind(moves.len() as i32)
    .bind(pgn)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM position_moves WHERE game_id = $1 AND ply > $2")
        .bind(&game_id)
        .bind(moves.len() as i32)
        .execute(&mut *tx)
        .await?;
    if !ids.is_empty() {
        sqlx::query(crate::import::POSITION_UPSERT)
            .bind(&ids)
            .bind(&plies)
            .bind(&session.username)
            .bind(&epds)
            .bind(&ucis)
            .bind(&sans)
            .bind(&setups)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Writes memory for every session that predates the feature (or lost it), in the background.
pub fn spawn_memory_backfill(state: AppState) {
    tokio::spawn(async move {
        let ids: Vec<Uuid> = match sqlx::query_scalar(
            "SELECT s.id FROM sessions s WHERE s.remember AND NOT EXISTS (SELECT 1 FROM games g WHERE g.id = 'yvy:session:' || s.id::text)",
        )
        .fetch_all(&state.pool)
        .await
        {
            Ok(ids) => ids,
            Err(e) => {
                tracing::error!(error = %e, "could not list sessions for the memory backfill");
                return;
            }
        };
        for id in ids {
            if let Err(e) = sync_memory(&state, id).await {
                tracing::warn!(session = %id, error = %e, "memory backfill failed for a session");
            }
        }
    });
}

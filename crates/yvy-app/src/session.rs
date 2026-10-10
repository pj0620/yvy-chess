//! Games played against the model. Every rule lives here: validating moves, answering from
//! the player's history, asking the engine (or handing the side over) when there is none,
//! steering back toward familiar positions, undo, resignation and the end of the game.
//!
//! The engine runs outside this crate (a Web Worker in the browser). When the model needs it,
//! an operation returns an [`EngineRequest`]; the caller runs the engine and passes the
//! [`EngineResult`] back, and the choice of move is made here.

use std::collections::HashMap;

use rand::seq::IndexedRandom;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use shakmaty::uci::UciMove;
use yvy_core::{choose_index, decode_uci_string, encode_uci, setup_hash, successor_hashes, successors, Color, GameState, Mode, PlayerMove, Status};

use crate::{
    index::PlayerIndex,
    types::{AppError, AppResult, Filters, GameMeta},
};

/// Stockfish's UCI_Elo range; below it, Skill Level approximates weaker play.
pub const ENGINE_ELO_MIN: u32 = 1320;
pub const ENGINE_ELO_MAX: u32 = 3190;
/// Think time per engine move.
pub const ENGINE_MOVETIME_MS: u32 = 300;
/// How many steering candidates the engine evaluates.
const MAX_STEER_CANDIDATES: usize = 8;

/// The human took over `color` after `ply` moves because the model had no history there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Switch {
    pub ply: i32,
    pub color: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionMove {
    pub ply: i32,
    pub uci: String,
    pub san: String,
    /// "player" (the human), "history" (one of the player's past moves) or "engine".
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

/// A game against the model, exactly as persisted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub username: String,
    /// The side the human started with.
    pub user_color: String,
    pub engine_elo: i32,
    pub mode: Mode,
    #[serde(default)]
    pub filters: Filters,
    pub steer_cp: i32,
    pub remember: bool,
    /// "stockfish" or "switch": what happens when the model has no history.
    pub gap_mode: String,
    /// The side the human plays right now.
    pub current_color: String,
    #[serde(default)]
    pub switches: Vec<Switch>,
    pub status: String,
    pub result: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub moves: Vec<SessionMove>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CreateSession {
    pub username: String,
    pub user_color: String,
    #[serde(default)]
    pub engine_elo: Option<u32>,
    #[serde(default)]
    pub mode: Option<Mode>,
    #[serde(default)]
    pub filters: Option<Filters>,
    #[serde(default)]
    pub steer_cp: Option<u32>,
    #[serde(default)]
    pub remember: Option<bool>,
    #[serde(default)]
    pub gap_mode: Option<String>,
    /// chess.com ratings by time class, used for the default engine strength.
    #[serde(default)]
    pub ratings: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CandidateOut {
    pub uci: String,
    pub san: String,
    pub count: i64,
    pub game_ids: Vec<String>,
    pub last_played: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SteerCandidate {
    pub uci: String,
    pub san: String,
    pub games_ahead: i64,
    pub known_replies: i64,
    pub setup_games: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReturningMove {
    pub uci: String,
    pub san: String,
    pub games: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Coverage {
    pub in_history: bool,
    pub games: i64,
    pub candidates: Vec<CandidateOut>,
    pub returning_moves: Vec<ReturningMove>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionState {
    pub id: String,
    pub username: String,
    pub user_color: String,
    pub engine_elo: i32,
    pub mode: Mode,
    pub filters: Filters,
    pub steer_cp: i32,
    pub remember: bool,
    pub gap_mode: String,
    pub current_color: String,
    pub switches: Vec<Switch>,
    pub status: String,
    pub result: Option<String>,
    pub fen: String,
    pub turn: String,
    pub ply: u32,
    pub legal_moves: Vec<String>,
    pub moves: Vec<SessionMove>,
    pub coverage: Coverage,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub user_color: String,
    pub engine_elo: i32,
    pub mode: Mode,
    pub steer_cp: i32,
    pub remember: bool,
    pub gap_mode: String,
    pub current_color: String,
    pub status: String,
    pub result: Option<String>,
    pub plies: usize,
    pub created_at: i64,
    pub updated_at: i64,
}

/// What the engine should do for the model's next move.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EngineRequest {
    pub session_id: String,
    pub fen: String,
    pub movetime_ms: u32,
    /// The requested strength, after clamping to the engine's range.
    pub elo: u32,
    /// Set when strength is limited with UCI_LimitStrength / UCI_Elo.
    pub uci_elo: Option<u32>,
    /// Set when the target is below the Elo range and Skill Level is used instead.
    pub skill_level: Option<u32>,
    /// Moves to score alongside the best move, for steering. Empty when steering is off.
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineLine {
    pub uci: String,
    #[serde(default)]
    pub depth: Option<u32>,
    #[serde(default)]
    pub score_cp: Option<i32>,
    #[serde(default)]
    pub score_mate: Option<i32>,
}

/// The engine's answer to an [`EngineRequest`]. `lines` holds scores for the requested
/// candidates (when the best move was not one of them); `error` means no engine was available.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineResult {
    pub fen: String,
    #[serde(default)]
    pub best: Option<EngineLine>,
    #[serde(default)]
    pub lines: Vec<EngineLine>,
    #[serde(default)]
    pub error: Option<String>,
}

/// The state after an operation, plus the engine work needed before the model can move.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Step {
    pub state: SessionState,
    pub engine_request: Option<EngineRequest>,
}

pub fn parse_color(s: &str) -> AppResult<Color> {
    match s {
        "white" => Ok(Color::White),
        "black" => Ok(Color::Black),
        other => Err(AppError::BadRequest(format!("user_color must be \"white\" or \"black\", got \"{other}\""))),
    }
}

pub fn color_str(c: Color) -> &'static str {
    match c {
        Color::White => "white",
        Color::Black => "black",
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
pub fn default_elo(ratings: Option<&Value>, filters: &Filters) -> u32 {
    let preferred: Vec<&str> = match &filters.time_classes {
        Some(v) if !v.is_empty() => v.iter().map(String::as_str).collect(),
        _ => vec!["rapid", "blitz", "bullet", "daily"],
    };
    for class in preferred {
        if let Some(r) = ratings.and_then(|r| r.get(class)).and_then(Value::as_u64) {
            return r as u32;
        }
    }
    1500
}

/// (Elo used, UCI_Elo, Skill Level) for a target strength.
pub fn strength(elo: i32) -> (u32, Option<u32>, Option<u32>) {
    let elo = elo.max(0) as u32;
    if elo >= ENGINE_ELO_MIN {
        let used = elo.min(ENGINE_ELO_MAX);
        (used, Some(used), None)
    } else {
        let span = (ENGINE_ELO_MIN as i64 - 600).max(1);
        let skill = ((elo as i64 - 600).max(0) * 19 / span).clamp(0, 19) as u32;
        (elo, None, Some(skill))
    }
}

pub fn session_game_id(id: &str) -> String {
    format!("yvy:session:{id}")
}

pub fn new_session(id: String, req: CreateSession, now: i64) -> AppResult<Session> {
    let username = req.username.trim().to_ascii_lowercase();
    let user_color = color_str(parse_color(&req.user_color)?).to_string();
    let gap_mode = match req.gap_mode.as_deref().unwrap_or("stockfish") {
        g @ ("stockfish" | "switch") => g.to_string(),
        other => return Err(AppError::BadRequest(format!("gap_mode must be \"stockfish\" or \"switch\", got \"{other}\""))),
    };
    let filters = req.filters.unwrap_or_default().normalized();
    let engine_elo = req.engine_elo.unwrap_or_else(|| default_elo(req.ratings.as_ref(), &filters)) as i32;
    Ok(Session {
        id,
        username,
        current_color: user_color.clone(),
        user_color,
        engine_elo,
        mode: req.mode.unwrap_or_default(),
        filters,
        steer_cp: req.steer_cp.unwrap_or(100).min(2000) as i32,
        remember: req.remember.unwrap_or(true),
        gap_mode,
        switches: Vec::new(),
        status: "playing".to_string(),
        result: None,
        created_at: now,
        updated_at: now,
        moves: Vec::new(),
    })
}

pub fn replay(s: &Session) -> AppResult<GameState> {
    let ucis: Vec<&str> = s.moves.iter().map(|m| m.uci.as_str()).collect();
    GameState::from_uci_moves(&ucis).map_err(|e| AppError::Internal(format!("stored moves of game {} do not replay: {e}", s.id)))
}

/// The player's past moves from the current position, as shown and as sampled from. Moves that
/// are not legal here (a hash collision, in theory) are dropped.
pub fn candidates_out(idx: &PlayerIndex, game: &GameState, filters: &Filters) -> Vec<CandidateOut> {
    idx.candidates(game.hash(), filters)
        .into_iter()
        .filter_map(|c| {
            let uci = decode_uci_string(c.mv);
            let san = game.san_of(&uci)?;
            Some(CandidateOut {
                uci,
                san,
                count: c.count,
                game_ids: c.games.iter().map(|&g| idx.game(g).id.clone()).collect(),
                last_played: c.last_played,
            })
        })
        .collect()
}

/// Every legal move for the side to move that leads back toward the player's games, strongest
/// first. `games_ahead` counts past games reached one opponent reply later (move-order
/// transpositions); `setup_games` counts past decisions after which the mover's own pieces
/// stood exactly as they would after this move.
pub fn steering_candidates(idx: &PlayerIndex, game: &GameState, filters: &Filters) -> Vec<SteerCandidate> {
    let mover = game.turn();
    let mut out: Vec<SteerCandidate> = successors(game.position())
        .into_iter()
        .filter_map(|first| {
            let mut games = 0i64;
            let mut known = 0i64;
            for key in successor_hashes(&first.pos) {
                let n = idx.count(key, filters);
                if n > 0 {
                    games += n;
                    known += 1;
                }
            }
            let setup_games = idx.count_setup(setup_hash(&first.pos, mover), filters);
            (games > 0 || setup_games > 0).then_some(SteerCandidate {
                uci: first.uci,
                san: first.san,
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
    out
}

/// Moves for the side to move after which the position is one the player has faced.
pub fn returning_moves(idx: &PlayerIndex, game: &GameState, filters: &Filters) -> Vec<ReturningMove> {
    let mut out: Vec<ReturningMove> = successors(game.position())
        .into_iter()
        .filter_map(|f| {
            let n = idx.count(f.key, filters);
            (n > 0).then_some(ReturningMove { uci: f.uci, san: f.san, games: n })
        })
        .collect();
    out.sort_by(|a, b| b.games.cmp(&a.games).then(a.uci.cmp(&b.uci)));
    out
}

fn push_move(s: &mut Session, game: &mut GameState, uci: &str, source: &str, detail: Option<Value>) -> AppResult<()> {
    let applied = game
        .apply_uci(uci)
        .map_err(|e| AppError::Internal(format!("{source} move {uci} was rejected: {e}")))?;
    s.moves.push(SessionMove {
        ply: game.ply() as i32,
        uci: applied.uci,
        san: applied.san,
        source: source.to_string(),
        detail,
    });
    Ok(())
}

/// Plays the human's move.
pub fn play(s: &mut Session, game: &mut GameState, uci: &str) -> AppResult<()> {
    if s.status != "playing" {
        return Err(AppError::Conflict("the game is over".to_string()));
    }
    if game.turn() != parse_color(&s.current_color)? {
        return Err(AppError::Conflict("it is not your turn".to_string()));
    }
    let applied = game.apply_uci(uci.trim()).map_err(|e| AppError::BadRequest(e.to_string()))?;
    s.moves.push(SessionMove {
        ply: game.ply() as i32,
        uci: applied.uci,
        san: applied.san,
        source: "player".to_string(),
        detail: None,
    });
    Ok(())
}

/// If it is the model's turn: answers from the player's history, hands the side to the human
/// (switch mode), or returns the engine work needed.
pub fn reply_if_needed(idx: &PlayerIndex, s: &mut Session, game: &mut GameState) -> AppResult<Option<EngineRequest>> {
    if s.status != "playing" || game.status().is_over() || game.turn() == parse_color(&s.current_color)? {
        return Ok(None);
    }
    let candidates = candidates_out(idx, game, &s.filters);
    if !candidates.is_empty() {
        let counts: Vec<u32> = candidates.iter().map(|c| c.count.max(0) as u32).collect();
        let pick = choose_index(&counts, s.mode, &mut rand::rng()).expect("candidates are non-empty");
        let chosen = candidates[pick].clone();
        let total: i64 = candidates.iter().map(|c| c.count).sum();
        let detail = json!({
            "games": total,
            "chosen": chosen.uci,
            "chosen_count": chosen.count,
            "mode": s.mode,
            "candidates": candidates,
        });
        push_move(s, game, &chosen.uci, "history", Some(detail))?;
        return Ok(None);
    }
    if s.gap_mode == "switch" {
        let taken = color_str(game.turn());
        s.current_color = taken.to_string();
        s.switches.push(Switch { ply: game.ply() as i32, color: taken.to_string() });
        return Ok(None);
    }
    let (elo, uci_elo, skill_level) = strength(s.engine_elo);
    let candidates = if s.steer_cp > 0 {
        steering_candidates(idx, game, &s.filters)
            .into_iter()
            .take(MAX_STEER_CANDIDATES)
            .map(|c| c.uci)
            .collect()
    } else {
        Vec::new()
    };
    Ok(Some(EngineRequest {
        session_id: s.id.clone(),
        fen: game.fen(),
        movetime_ms: ENGINE_MOVETIME_MS,
        elo,
        uci_elo,
        skill_level,
        candidates,
    }))
}

/// Orders mates above any material evaluation, from the side to move's point of view.
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

/// Applies the engine's answer: Stockfish proposes its best move; if steering is on, the
/// candidate with the most games ahead (then the most familiar setup, then the better score)
/// within the tolerance of the best evaluation is played instead. Without an engine, the top
/// steering candidate or a random legal move is played and labelled as such.
pub fn apply_engine_result(idx: &PlayerIndex, s: &mut Session, game: &mut GameState, result: &EngineResult) -> AppResult<()> {
    let (elo, _, skill_level) = strength(s.engine_elo);
    let tolerance = s.steer_cp.max(0) as i64;
    let mut steering = if tolerance > 0 { steering_candidates(idx, game, &s.filters) } else { Vec::new() };
    steering.truncate(MAX_STEER_CANDIDATES);

    let best = match (&result.error, &result.best) {
        (None, Some(b)) if game.san_of(&b.uci).is_some() => b.clone(),
        _ => {
            let reason = result.error.clone().unwrap_or_else(|| "the engine returned no usable move".to_string());
            if let Some(c) = steering.first() {
                let detail = json!({
                    "engine": "random",
                    "reason": reason,
                    "steer": {
                        "tolerance_cp": tolerance,
                        "chosen_is_best": false,
                        "games_ahead": c.games_ahead,
                        "known_replies": c.known_replies,
                        "setup_games": c.setup_games,
                        "considered": considered(&steering, &HashMap::new()),
                    },
                });
                let uci = c.uci.clone();
                return push_move(s, game, &uci, "engine", Some(detail));
            }
            let legal = game.legal_moves_uci();
            let pick = legal
                .choose(&mut rand::rng())
                .cloned()
                .ok_or_else(|| AppError::Internal("no legal moves in a game that is not over".to_string()))?;
            return push_move(s, game, &pick, "engine", Some(json!({ "engine": "random", "reason": reason })));
        }
    };

    let mut detail = json!({
        "engine": "stockfish",
        "elo": elo,
        "skill_level": skill_level,
        "depth": best.depth,
        "score_cp": best.score_cp,
        "score_mate": best.score_mate,
        "movetime_ms": ENGINE_MOVETIME_MS,
    });
    if steering.is_empty() {
        if tolerance > 0 {
            detail["steer"] = json!({ "tolerance_cp": tolerance, "candidates": 0 });
        }
        return push_move(s, game, &best.uci, "engine", Some(detail));
    }
    let best_score = score_value(best.score_cp, best.score_mate);
    if let Some(c) = steering.iter().find(|c| c.uci == best.uci) {
        detail["steer"] = json!({
            "tolerance_cp": tolerance,
            "chosen_is_best": true,
            "games_ahead": c.games_ahead,
            "known_replies": c.known_replies,
            "setup_games": c.setup_games,
            "best_uci": best.uci,
            "best_score_cp": best.score_cp,
            "considered": considered(&steering, &HashMap::new()),
        });
        return push_move(s, game, &best.uci, "engine", Some(detail));
    }
    let scores: HashMap<String, (Option<i32>, Option<i32>)> =
        result.lines.iter().map(|l| (l.uci.clone(), (l.score_cp, l.score_mate))).collect();
    let chosen = steering
        .iter()
        .filter_map(|c| {
            let &(cp, mate) = scores.get(&c.uci)?;
            let score = score_value(cp, mate);
            (best_score - score <= tolerance).then_some((c, score))
        })
        .max_by_key(|(c, score)| (c.games_ahead, c.setup_games, *score))
        .map(|(c, _)| c.clone());
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
                "best_uci": best.uci,
                "best_score_cp": best.score_cp,
                "best_score_mate": best.score_mate,
                "considered": considered(&steering, &scores),
            });
            push_move(s, game, &c.uci, "engine", Some(detail))
        }
        None => {
            detail["steer"] = json!({
                "tolerance_cp": tolerance,
                "rejected": true,
                "best_uci": best.uci,
                "best_score_cp": best.score_cp,
                "considered": considered(&steering, &scores),
            });
            push_move(s, game, &best.uci, "engine", Some(detail))
        }
    }
}

/// Takes back the human's last move together with the reply it received, restoring the side
/// the human was playing at that point.
pub fn undo(s: &mut Session) -> AppResult<()> {
    if s.status == "resigned" {
        return Err(AppError::Conflict("cannot undo after resigning".to_string()));
    }
    let mut keep = s.moves.len();
    if s.moves.last().is_some_and(|m| m.source != "player") {
        keep -= 1;
    }
    if keep > 0 && s.moves[keep - 1].source == "player" {
        keep -= 1;
    }
    if keep == s.moves.len() {
        return Err(AppError::Conflict("nothing to undo".to_string()));
    }
    s.moves.truncate(keep);
    s.current_color = color_str(replay(s)?.turn()).to_string();
    s.switches.retain(|sw| sw.ply as usize <= keep);
    s.status = "playing".to_string();
    s.result = None;
    Ok(())
}

pub fn resign(s: &mut Session) -> AppResult<()> {
    if s.status != "playing" {
        return Err(AppError::Conflict("the game is already over".to_string()));
    }
    s.status = "resigned".to_string();
    s.result = Some("loss".to_string());
    Ok(())
}

/// Records the outcome once the board says the game is over.
pub fn settle(s: &mut Session, game: &GameState) -> AppResult<()> {
    let status = game.status();
    if s.status != "playing" || !status.is_over() {
        return Ok(());
    }
    let human = parse_color(&s.current_color)?;
    s.status = status_str(status).to_string();
    s.result = Some(
        match game.winner() {
            Some(w) if w == human => "win",
            Some(_) => "loss",
            None => "draw",
        }
        .to_string(),
    );
    Ok(())
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

/// The human's decisions in this game, as a game of their history (time class `yvy`), or
/// `None` when the session is not remembered. The model's moves are never included.
pub fn memory(s: &Session) -> AppResult<Option<(GameMeta, Vec<PlayerMove>)>> {
    if !s.remember {
        return Ok(None);
    }
    let mut replay_game = GameState::new();
    let mut decisions = Vec::new();
    for m in &s.moves {
        let before = replay_game.hash();
        let mover = replay_game.turn();
        replay_game
            .apply_uci(&m.uci)
            .map_err(|e| AppError::Internal(format!("stored moves of game {} do not replay: {e}", s.id)))?;
        if m.source == "player" {
            let parsed: UciMove = m.uci.parse().map_err(|_| AppError::Internal(format!("bad stored move {}", m.uci)))?;
            decisions.push(PlayerMove {
                ply: m.ply as u32,
                key: before,
                setup: setup_hash(replay_game.position(), mover),
                mv: encode_uci(&parsed),
            });
        }
    }
    let meta = GameMeta {
        id: session_game_id(&s.id),
        color: s.user_color.clone(),
        time_class: "yvy".to_string(),
        rated: false,
        end_time: s.updated_at,
        result: memory_result(&s.status, s.result.as_deref()).to_string(),
        opponent: Some(format!("{} (model)", s.username)),
        player_rating: None,
        opponent_rating: None,
        eco: None,
        ply_count: s.moves.len() as u32,
    };
    Ok(Some((meta, decisions)))
}

pub fn state(idx: &PlayerIndex, s: &Session, game: &GameState) -> AppResult<SessionState> {
    let candidates = candidates_out(idx, game, &s.filters);
    let games = candidates.iter().map(|c| c.count).sum();
    let playing = s.status == "playing";
    let returning = if playing && game.turn() == parse_color(&s.current_color)? {
        returning_moves(idx, game, &s.filters)
    } else {
        Vec::new()
    };
    Ok(SessionState {
        id: s.id.clone(),
        username: s.username.clone(),
        user_color: s.user_color.clone(),
        engine_elo: s.engine_elo,
        mode: s.mode,
        filters: s.filters.clone(),
        steer_cp: s.steer_cp,
        remember: s.remember,
        gap_mode: s.gap_mode.clone(),
        current_color: s.current_color.clone(),
        switches: s.switches.clone(),
        status: s.status.clone(),
        result: s.result.clone(),
        fen: game.fen(),
        turn: color_str(game.turn()).to_string(),
        ply: game.ply(),
        legal_moves: if playing { game.legal_moves_uci() } else { Vec::new() },
        moves: s.moves.clone(),
        coverage: Coverage { in_history: !candidates.is_empty(), games, candidates, returning_moves: returning },
        created_at: s.created_at,
        updated_at: s.updated_at,
    })
}

pub fn summary(s: &Session) -> SessionSummary {
    SessionSummary {
        id: s.id.clone(),
        user_color: s.user_color.clone(),
        engine_elo: s.engine_elo,
        mode: s.mode,
        steer_cp: s.steer_cp,
        remember: s.remember,
        gap_mode: s.gap_mode.clone(),
        current_color: s.current_color.clone(),
        status: s.status.clone(),
        result: s.result.clone(),
        plies: s.moves.len(),
        created_at: s.created_at,
        updated_at: s.updated_at,
    }
}

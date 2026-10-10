//! Everything the app holds in memory: one index per player and the games played in the app.
//! The browser persists month blobs and session JSON and feeds them back in on load.

use std::collections::HashMap;

use yvy_core::GameState;

use crate::{
    import::{self, ImportedMonth},
    index::{PlayerIndex, Summary},
    pgn,
    session::{self, CandidateOut, CreateSession, EngineRequest, EngineResult, Session, SessionState, SessionSummary, Step},
    types::{AppError, AppResult, Filters},
};

pub fn normalize_username(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

#[derive(Debug, Default)]
pub struct Store {
    players: HashMap<String, PlayerIndex>,
    sessions: HashMap<String, Session>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn has_player(&self, username: &str) -> bool {
        self.players.contains_key(&normalize_username(username))
    }

    pub fn index(&self, username: &str) -> Option<&PlayerIndex> {
        self.players.get(&normalize_username(username))
    }

    fn player_mut(&mut self, username: &str) -> &mut PlayerIndex {
        let idx = self.players.entry(normalize_username(username)).or_default();
        idx.prepare();
        idx
    }

    /// Indexes one month from chess.com and returns the blob to persist for it. The blob holds
    /// every usable game of the month, so it can replace an earlier copy of the same month.
    pub fn import_month(&mut self, username: &str, json: &str) -> AppResult<ImportedMonth> {
        let username = normalize_username(username);
        let parsed = import::parse_month(&username, json)?;
        let idx = self.players.entry(username).or_default();
        let mut out = ImportedMonth {
            seen: parsed.seen,
            skipped: parsed.skipped,
            errors: parsed.errors,
            ..Default::default()
        };
        for g in &parsed.games {
            if idx.add_game(g) {
                out.new += 1;
                out.positions += g.decisions.len() as u64;
            }
        }
        out.blob = import::encode_blob(&parsed.games);
        Ok(out)
    }

    /// Loads a month persisted by [`Store::import_month`]. Returns how many games were new.
    pub fn load_month(&mut self, username: &str, blob: &[u8]) -> AppResult<u64> {
        let games = import::decode_blob(blob)?;
        let idx = self.players.entry(normalize_username(username)).or_default();
        Ok(games.iter().filter(|g| idx.add_game(g)).count() as u64)
    }

    /// Makes a player's index ready for queries after a batch of imports or loads.
    pub fn finish_loading(&mut self, username: &str) {
        self.player_mut(username);
    }

    pub fn forget_player(&mut self, username: &str) {
        let username = normalize_username(username);
        self.players.remove(&username);
        self.sessions.retain(|_, s| s.username != username);
    }

    pub fn player_summary(&mut self, username: &str) -> Summary {
        self.player_mut(username).summary()
    }

    /// What the player did from the position in `fen`.
    pub fn position(&mut self, username: &str, fen: &str, filters: &Filters) -> AppResult<Vec<CandidateOut>> {
        let game = GameState::from_fen(fen).map_err(AppError::BadRequest)?;
        let idx = self.player_mut(username);
        Ok(session::candidates_out(idx, &game, filters))
    }

    /// Restores a persisted session and its memory.
    pub fn load_session(&mut self, s: Session) -> AppResult<()> {
        session::replay(&s)?;
        let idx = self.player_mut(&s.username);
        if let Some((meta, decisions)) = session::memory(&s)? {
            idx.set_live_game(meta, decisions);
        }
        self.sessions.insert(s.id.clone(), s);
        Ok(())
    }

    pub fn has_session(&self, id: &str) -> bool {
        self.sessions.contains_key(id)
    }

    pub fn session(&self, id: &str) -> AppResult<&Session> {
        self.sessions.get(id).ok_or_else(|| AppError::NotFound("game not found".to_string()))
    }

    pub fn sessions_for(&self, username: &str) -> Vec<SessionSummary> {
        let username = normalize_username(username);
        let mut out: Vec<SessionSummary> =
            self.sessions.values().filter(|s| s.username == username).map(session::summary).collect();
        out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id)));
        out
    }

    /// Runs `f` on a session with its game and index, then records the outcome, updates the
    /// memory and returns the new state.
    fn with<F>(&mut self, id: &str, now: i64, f: F) -> AppResult<Step>
    where
        F: FnOnce(&PlayerIndex, &mut Session, &mut GameState) -> AppResult<Option<EngineRequest>>,
    {
        let s = self.sessions.get_mut(id).ok_or_else(|| AppError::NotFound("game not found".to_string()))?;
        let idx = self.players.entry(s.username.clone()).or_default();
        idx.prepare();
        let mut game = session::replay(s)?;
        let engine_request = f(idx, s, &mut game)?;
        session::settle(s, &game)?;
        s.updated_at = now;
        if let Some((meta, decisions)) = session::memory(s)? {
            idx.set_live_game(meta, decisions);
        }
        let state = session::state(idx, s, &game)?;
        Ok(Step { state, engine_request })
    }

    pub fn create_session(&mut self, id: &str, req: CreateSession, now: i64) -> AppResult<Step> {
        let username = normalize_username(&req.username);
        if !self.players.contains_key(&username) {
            return Err(AppError::NotFound(format!("{username} has not been imported yet")));
        }
        if self.sessions.contains_key(id) {
            return Err(AppError::Conflict(format!("a game with id {id} already exists")));
        }
        let s = session::new_session(id.to_string(), req, now)?;
        self.sessions.insert(id.to_string(), s);
        self.with(id, now, session::reply_if_needed)
    }

    pub fn play(&mut self, id: &str, uci: &str, now: i64) -> AppResult<Step> {
        self.with(id, now, |idx, s, game| {
            session::play(s, game, uci)?;
            session::reply_if_needed(idx, s, game)
        })
    }

    /// Applies an engine answer. An answer for a position the game is no longer in (the human
    /// undid a move meanwhile) is ignored and the current request, if any, is returned again.
    pub fn engine_result(&mut self, id: &str, result: &EngineResult, now: i64) -> AppResult<Step> {
        self.with(id, now, |idx, s, game| {
            let awaiting = s.status == "playing"
                && !game.status().is_over()
                && game.turn() != session::parse_color(&s.current_color)?;
            if !awaiting {
                return Ok(None);
            }
            if result.fen != game.fen() {
                return session::reply_if_needed(idx, s, game);
            }
            session::apply_engine_result(idx, s, game, result)?;
            Ok(None)
        })
    }

    /// Continues a game whose model move is still pending, for example after a reload.
    pub fn resume(&mut self, id: &str, now: i64) -> AppResult<Step> {
        self.with(id, now, session::reply_if_needed)
    }

    pub fn undo(&mut self, id: &str, now: i64) -> AppResult<Step> {
        self.with(id, now, |idx, s, game| {
            session::undo(s)?;
            *game = session::replay(s)?;
            session::reply_if_needed(idx, s, game)
        })
    }

    pub fn resign(&mut self, id: &str, now: i64) -> AppResult<Step> {
        self.with(id, now, |_, s, _| {
            session::resign(s)?;
            Ok(None)
        })
    }

    pub fn state(&mut self, id: &str) -> AppResult<SessionState> {
        let s = self.sessions.get(id).ok_or_else(|| AppError::NotFound("game not found".to_string()))?;
        let idx = self.players.entry(s.username.clone()).or_default();
        idx.prepare();
        session::state(idx, s, &session::replay(s)?)
    }

    pub fn pgn(&self, id: &str) -> AppResult<String> {
        Ok(pgn::render(self.session(id)?))
    }
}

use shakmaty::{san::SanPlus, uci::UciMove, CastlingMode, Chess, Color, Position};

use crate::{codec::position_hash, key::position_key};

/// A move that has been played in a live game.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct AppliedMove {
    pub uci: String,
    pub san: String,
}

/// Why a game is over, or `Playing` if it is not. Draws by repetition and the fifty-move
/// rule are adjudicated automatically rather than claimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Playing,
    Checkmate,
    Stalemate,
    InsufficientMaterial,
    ThreefoldRepetition,
    FiftyMoveRule,
}

impl Status {
    pub fn is_over(self) -> bool {
        self != Status::Playing
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GameError {
    #[error("illegal move: {0}")]
    IllegalMove(String),
    #[error("the game is over")]
    GameOver,
}

/// The authoritative state of one game from the standard start position.
#[derive(Debug, Clone)]
pub struct GameState {
    pos: Chess,
    moves: Vec<AppliedMove>,
    /// Hashes of every position seen so far, starting with the initial one.
    keys: Vec<u64>,
}

impl Default for GameState {
    fn default() -> Self {
        Self::new()
    }
}

impl GameState {
    pub fn new() -> Self {
        let pos = Chess::default();
        let keys = vec![position_hash(&pos)];
        GameState { pos, moves: Vec::new(), keys }
    }

    /// A position given as FEN, for lookups; such a game has no move history.
    pub fn from_fen(fen: &str) -> Result<Self, String> {
        let fen: shakmaty::fen::Fen = fen.trim().parse().map_err(|e| format!("invalid FEN: {e}"))?;
        let pos: Chess = fen.into_position(CastlingMode::Standard).map_err(|e| format!("illegal position: {e}"))?;
        let keys = vec![position_hash(&pos)];
        Ok(GameState { pos, moves: Vec::new(), keys })
    }

    /// Rebuilds a game from UCI moves, validating each of them.
    pub fn from_uci_moves<S: AsRef<str>>(moves: &[S]) -> Result<Self, GameError> {
        let mut game = Self::new();
        for uci in moves {
            game.apply_uci(uci.as_ref())?;
        }
        Ok(game)
    }

    /// Plays a move given in UCI notation, rejecting illegal moves and moves after the end.
    pub fn apply_uci(&mut self, uci: &str) -> Result<AppliedMove, GameError> {
        if self.status().is_over() {
            return Err(GameError::GameOver);
        }
        let parsed = uci
            .parse::<UciMove>()
            .map_err(|_| GameError::IllegalMove(uci.to_string()))?;
        let m = parsed
            .to_move(&self.pos)
            .map_err(|_| GameError::IllegalMove(uci.to_string()))?;
        let san = SanPlus::from_move_and_play_unchecked(&mut self.pos, m).to_string();
        let applied = AppliedMove {
            uci: m.to_uci(CastlingMode::Standard).to_string(),
            san,
        };
        self.moves.push(applied.clone());
        self.keys.push(position_hash(&self.pos));
        Ok(applied)
    }

    /// Takes back the last move. Returns `None` if no move has been played.
    pub fn undo(&mut self) -> Option<AppliedMove> {
        let last = self.moves.pop()?;
        let remaining: Vec<String> = self.moves.iter().map(|m| m.uci.clone()).collect();
        *self = Self::from_uci_moves(&remaining).expect("previously validated moves replay cleanly");
        Some(last)
    }

    pub fn moves(&self) -> &[AppliedMove] {
        &self.moves
    }

    /// The current position.
    pub fn position(&self) -> &Chess {
        &self.pos
    }

    pub fn ply(&self) -> u32 {
        self.moves.len() as u32
    }

    pub fn turn(&self) -> Color {
        self.pos.turn()
    }

    pub fn fen(&self) -> String {
        shakmaty::fen::Fen::from_position(&self.pos, shakmaty::EnPassantMode::Legal).to_string()
    }

    /// Key of the current position, see [`position_key`].
    pub fn key(&self) -> String {
        position_key(&self.pos)
    }

    /// Hash of the current position, see [`position_hash`].
    pub fn hash(&self) -> u64 {
        position_hash(&self.pos)
    }

    /// SAN of a legal UCI move in the current position, without playing it.
    pub fn san_of(&self, uci: &str) -> Option<String> {
        let m = uci.parse::<UciMove>().ok()?.to_move(&self.pos).ok()?;
        Some(SanPlus::from_move(self.pos.clone(), m).to_string())
    }

    pub fn legal_moves_uci(&self) -> Vec<String> {
        self.pos
            .legal_moves()
            .iter()
            .map(|m| m.to_uci(CastlingMode::Standard).to_string())
            .collect()
    }

    pub fn status(&self) -> Status {
        if self.pos.is_checkmate() {
            Status::Checkmate
        } else if self.pos.is_stalemate() {
            Status::Stalemate
        } else if self.pos.is_insufficient_material() {
            Status::InsufficientMaterial
        } else if self.repetitions() >= 3 {
            Status::ThreefoldRepetition
        } else if self.pos.halfmoves() >= 100 {
            Status::FiftyMoveRule
        } else {
            Status::Playing
        }
    }

    /// The side that delivered checkmate, if any.
    pub fn winner(&self) -> Option<Color> {
        if self.pos.is_checkmate() {
            Some(!self.pos.turn())
        } else {
            None
        }
    }

    fn repetitions(&self) -> usize {
        let current = self.keys.last().expect("at least the start position");
        self.keys.iter().filter(|k| *k == current).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_has_twenty_legal_moves() {
        let g = GameState::new();
        assert_eq!(g.legal_moves_uci().len(), 20);
        assert_eq!(g.status(), Status::Playing);
        assert_eq!(g.turn(), Color::White);
        assert_eq!(g.fen(), "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
    }

    #[test]
    fn rejects_illegal_and_malformed_moves() {
        let mut g = GameState::new();
        assert_eq!(g.apply_uci("e2e5"), Err(GameError::IllegalMove("e2e5".into())));
        assert_eq!(g.apply_uci("nonsense"), Err(GameError::IllegalMove("nonsense".into())));
        assert_eq!(g.ply(), 0);
    }

    #[test]
    fn scholars_mate_ends_the_game() {
        let mut g = GameState::new();
        for uci in ["e2e4", "e7e5", "f1c4", "b8c6", "d1h5", "g8f6", "h5f7"] {
            g.apply_uci(uci).unwrap();
        }
        assert_eq!(g.status(), Status::Checkmate);
        assert_eq!(g.winner(), Some(Color::White));
        assert_eq!(g.moves().last().unwrap().san, "Qxf7#");
        assert!(g.legal_moves_uci().is_empty());
        assert_eq!(g.apply_uci("e8f7"), Err(GameError::GameOver));
    }

    #[test]
    fn castling_is_expressed_as_king_moves() {
        let mut g = GameState::new();
        for uci in ["e2e4", "e7e5", "g1f3", "b8c6", "f1c4", "f8c5"] {
            g.apply_uci(uci).unwrap();
        }
        assert!(g.legal_moves_uci().contains(&"e1g1".to_string()));
        let applied = g.apply_uci("e1g1").unwrap();
        assert_eq!(applied.san, "O-O");
        assert_eq!(applied.uci, "e1g1");
    }

    #[test]
    fn threefold_repetition_is_a_draw() {
        let mut g = GameState::new();
        for uci in ["g1f3", "g8f6", "f3g1", "f6g8", "g1f3", "g8f6", "f3g1"] {
            g.apply_uci(uci).unwrap();
            assert_eq!(g.status(), Status::Playing);
        }
        g.apply_uci("f6g8").unwrap();
        assert_eq!(g.status(), Status::ThreefoldRepetition);
    }

    #[test]
    fn undo_restores_the_previous_position() {
        let mut g = GameState::new();
        g.apply_uci("e2e4").unwrap();
        g.apply_uci("e7e5").unwrap();
        let undone = g.undo().unwrap();
        assert_eq!(undone.uci, "e7e5");
        assert_eq!(g.ply(), 1);
        assert_eq!(g.turn(), Color::Black);
        assert_eq!(g.key(), "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq -");
        let rebuilt = GameState::from_uci_moves(&["e2e4"]).unwrap();
        assert_eq!(rebuilt.fen(), g.fen());
        assert!(GameState::new().undo().is_none());
    }
}

use std::ops::ControlFlow;

use pgn_reader::{RawTag, Reader, SanPlus, Visitor};
use shakmaty::{fen::Fen, CastlingMode, Chess, Color, Position};

use crate::codec::{encode_move, position_hash, setup_hash};

/// One decision the player made in a game: the position they faced and what they played.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlayerMove {
    /// 1-based ply within the game (1 is White's first move, 2 is Black's first move).
    pub ply: u32,
    /// Hash of the position *before* the move, see [`position_hash`].
    pub key: u64,
    /// Hash of the player's own piece setup *after* the move, see [`setup_hash`].
    pub setup: u64,
    /// The move, see [`crate::encode_move`].
    pub mv: u16,
}

impl PlayerMove {
    pub fn uci(&self) -> String {
        crate::decode_uci_string(self.mv)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Extracted {
    pub moves: Vec<PlayerMove>,
    /// Total number of plies in the game, both sides.
    pub ply_count: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum PgnError {
    #[error("no game found in PGN text")]
    Empty,
    #[error("invalid FEN tag: {0}")]
    BadFen(String),
    #[error("illegal move at ply {ply}: {san}")]
    IllegalMove { ply: u32, san: String },
    #[error("could not read PGN: {0}")]
    Io(#[from] std::io::Error),
}

/// Replays the first game in `pgn` and returns every move made by the side `player`,
/// keyed by the position it was played from. Comments, clock annotations, NAGs and
/// variations are ignored. Games that start from a `[FEN "..."]` tag are supported.
pub fn extract_player_moves(pgn: &str, player: Color) -> Result<Extracted, PgnError> {
    let mut reader = Reader::new(pgn.as_bytes());
    let mut visitor = Extractor { player };
    match reader.read_game(&mut visitor)? {
        Some(output) => output,
        None => Err(PgnError::Empty),
    }
}

struct Extractor {
    player: Color,
}

struct Movetext {
    pos: Chess,
    ply: u32,
    moves: Vec<PlayerMove>,
}

impl Visitor for Extractor {
    type Tags = Option<Chess>;
    type Movetext = Movetext;
    type Output = Result<Extracted, PgnError>;

    fn begin_tags(&mut self) -> ControlFlow<Self::Output, Self::Tags> {
        ControlFlow::Continue(None)
    }

    fn tag(&mut self, tags: &mut Self::Tags, name: &[u8], value: RawTag<'_>) -> ControlFlow<Self::Output> {
        if name == b"FEN" {
            let parsed = Fen::from_ascii(value.as_bytes())
                .map_err(|e| PgnError::BadFen(e.to_string()))
                .and_then(|fen| {
                    fen.into_position(CastlingMode::Standard)
                        .map_err(|e| PgnError::BadFen(e.to_string()))
                });
            match parsed {
                Ok(pos) => *tags = Some(pos),
                Err(e) => return ControlFlow::Break(Err(e)),
            }
        }
        ControlFlow::Continue(())
    }

    fn begin_movetext(&mut self, tags: Self::Tags) -> ControlFlow<Self::Output, Self::Movetext> {
        ControlFlow::Continue(Movetext {
            pos: tags.unwrap_or_default(),
            ply: 0,
            moves: Vec::new(),
        })
    }

    fn san(&mut self, mt: &mut Self::Movetext, san_plus: SanPlus) -> ControlFlow<Self::Output> {
        let m = match san_plus.san.to_move(&mt.pos) {
            Ok(m) => m,
            Err(_) => {
                return ControlFlow::Break(Err(PgnError::IllegalMove {
                    ply: mt.ply + 1,
                    san: san_plus.to_string(),
                }))
            }
        };
        let players_turn = mt.pos.turn() == self.player;
        let key = players_turn.then(|| position_hash(&mt.pos));
        mt.pos.play_unchecked(m);
        mt.ply += 1;
        if let Some(key) = key {
            mt.moves.push(PlayerMove {
                ply: mt.ply,
                key,
                setup: setup_hash(&mt.pos, self.player),
                mv: encode_move(m),
            });
        }
        ControlFlow::Continue(())
    }

    fn end_game(&mut self, mt: Self::Movetext) -> Self::Output {
        Ok(Extracted {
            moves: mt.moves,
            ply_count: mt.ply,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shaped like a chess.com export: clock comments after every move and Black's moves
    /// repeated with "1..." numbering.
    const SCHOLARS_MATE: &str = r#"[Event "Live Chess"]
[Site "Chess.com"]
[Date "2024.01.01"]
[White "alice"]
[Black "bob"]
[Result "1-0"]
[TimeControl "180"]

1. e4 {[%clk 0:02:59]} 1... e5 {[%clk 0:02:58]} 2. Bc4 {[%clk 0:02:57]} 2... Nc6 {[%clk 0:02:56]} 3. Qh5 {[%clk 0:02:55]} 3... Nf6 {[%clk 0:02:54]} 4. Qxf7# {[%clk 0:02:53]} 1-0
"#;

    #[test]
    fn extracts_only_the_players_moves() {
        use crate::{own_setup_key, GameState};
        let white = extract_player_moves(SCHOLARS_MATE, Color::White).unwrap();
        assert_eq!(white.ply_count, 7);
        let ucis: Vec<_> = white.moves.iter().map(|m| m.uci()).collect();
        assert_eq!(ucis, ["e2e4", "f1c4", "d1h5", "h5f7"]);
        let plies: Vec<_> = white.moves.iter().map(|m| m.ply).collect();
        assert_eq!(plies, [1, 3, 5, 7]);
        assert_eq!(white.moves[0].key, GameState::new().hash());
        let after_e4 = GameState::from_uci_moves(&["e2e4"]).unwrap();
        assert_eq!(white.moves[0].setup, crate::setup_hash(after_e4.position(), Color::White));
        assert_eq!(own_setup_key(after_e4.position(), Color::White), "8/8/8/8/4P3/8/PPPP1PPP/RNBQKBNR");

        let black = extract_player_moves(SCHOLARS_MATE, Color::Black).unwrap();
        let ucis: Vec<_> = black.moves.iter().map(|m| m.uci()).collect();
        assert_eq!(ucis, ["e7e5", "b8c6", "g8f6"]);
        assert_eq!(black.moves[0].key, after_e4.hash());
    }

    #[test]
    fn illegal_moves_are_reported_with_their_ply() {
        let err = extract_player_moves("1. e4 e5 2. Nf3 Nc6 3. Ke8 *", Color::White).unwrap_err();
        match err {
            PgnError::IllegalMove { ply, san } => {
                assert_eq!(ply, 5);
                assert_eq!(san, "Ke8");
            }
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn empty_input_is_an_error() {
        assert!(matches!(extract_player_moves("   \n", Color::White), Err(PgnError::Empty)));
    }

    #[test]
    fn game_with_no_moves_extracts_nothing() {
        let out = extract_player_moves("[Event \"x\"]\n\n*\n", Color::White).unwrap();
        assert!(out.moves.is_empty());
        assert_eq!(out.ply_count, 0);
    }
}

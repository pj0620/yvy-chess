//! Pure chess logic for You v You Chess: position keys, PGN replay, move sampling and the
//! state of a live game. No IO, no async, no database, so everything here is unit-testable.

pub mod game;
pub mod key;
pub mod lookahead;
pub mod pgn;
pub mod sample;

pub use game::{AppliedMove, GameError, GameState, Status};
pub use key::{own_setup_key, position_key};
pub use lookahead::{successors, Successor};
pub use pgn::{extract_player_moves, Extracted, PgnError, PlayerMove};
pub use sample::{choose_index, Mode};
pub use shakmaty::Color;

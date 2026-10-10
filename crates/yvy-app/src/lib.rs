//! Application logic for You v You Chess, independent of where it runs. The browser build
//! wraps [`Store`] with wasm-bindgen; everything here is plain Rust and tested natively.
//!
//! The model of "you" is an index of every decision the player made: the position they faced
//! (as a 64-bit hash) and the move they chose. Imported games live in a sorted array that is
//! built once per load; games played in the app live in a small mutable overlay so that undo
//! can take a memory back.

pub mod import;
pub mod index;
pub mod pgn;
pub mod session;
pub mod store;
pub mod types;

pub use import::ImportedMonth;
pub use index::{PlayerIndex, Summary};
pub use session::{
    CandidateOut, CreateSession, EngineLine, EngineRequest, EngineResult, ReturningMove, Session, SessionState, SessionSummary,
    SteerCandidate, Step, Switch,
};
pub use store::Store;
pub use types::{AppError, AppResult, Filters, GameMeta, StoredGame};

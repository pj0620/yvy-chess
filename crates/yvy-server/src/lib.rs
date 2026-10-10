//! You v You Chess server: imports a chess.com account, indexes every decision the player
//! made, and lets them play against that index with Stockfish filling the gaps.

pub mod api;
pub mod auth;
pub mod chesscom;
pub mod config;
pub mod db;
pub mod engine;
pub mod error;
pub mod import;
pub mod model;
pub mod session;
pub mod state;

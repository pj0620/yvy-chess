use std::{net::SocketAddr, path::PathBuf};

use anyhow::Context;

#[derive(Clone, Debug)]
pub struct Config {
    pub database_url: String,
    pub bind: SocketAddr,
    /// Path or command name of the Stockfish binary.
    pub stockfish_path: String,
    /// Directory with the built web bundle. When unset, only the API is served.
    pub web_dir: Option<PathBuf>,
    /// Contact string for the chess.com User-Agent header.
    pub contact: String,
    /// How long Stockfish thinks per move.
    pub engine_movetime_ms: u64,
    /// When set, every request except the health check needs HTTP Basic auth with this password.
    pub password: Option<String>,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://yvy:yvy@localhost:5432/yvy".to_string());
        let bind = std::env::var("YVY_BIND")
            .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
            .parse()
            .context("YVY_BIND must be host:port")?;
        let stockfish_path = std::env::var("STOCKFISH_PATH").unwrap_or_else(|_| "stockfish".to_string());
        let web_dir = std::env::var("YVY_WEB_DIR")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from);
        let contact = std::env::var("YVY_CONTACT").unwrap_or_else(|_| "unknown".to_string());
        let engine_movetime_ms = std::env::var("YVY_ENGINE_MOVETIME_MS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300);
        let password = std::env::var("YVY_PASSWORD").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        Ok(Config {
            database_url,
            bind,
            stockfish_path,
            web_dir,
            contact,
            engine_movetime_ms,
            password,
        })
    }

    /// Configuration for integration tests: no engine, no web bundle.
    pub fn for_tests() -> Self {
        Config {
            database_url: String::new(),
            bind: "127.0.0.1:0".parse().unwrap(),
            stockfish_path: "/nonexistent/stockfish".to_string(),
            web_dir: None,
            contact: "tests".to_string(),
            engine_movetime_ms: 50,
            password: None,
        }
    }
}

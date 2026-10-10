//! Minimal client for the chess.com public API (https://www.chess.com/news/view/published-data-api).
//! Requests are made one at a time, with a User-Agent that identifies us, and retried with
//! backoff when chess.com throttles.

use std::time::Duration;

use reqwest::{header, StatusCode};
use serde::Deserialize;
use serde_json::Value;

pub const DEFAULT_BASE: &str = "https://api.chess.com/pub";

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("chess.com returned HTTP {status} for {url}")]
    Status { status: u16, url: String },
}

#[derive(Debug, Clone, Deserialize)]
pub struct Player {
    pub player_id: Option<i64>,
    pub username: Option<String>,
    /// Unix timestamp of account creation.
    pub joined: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Side {
    pub username: String,
    pub rating: Option<i32>,
    /// chess.com result code: win, checkmated, resigned, timeout, agreed, repetition, ...
    pub result: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Game {
    pub url: String,
    #[serde(default)]
    pub pgn: Option<String>,
    pub time_class: String,
    #[serde(default)]
    pub time_control: Option<String>,
    pub rules: String,
    #[serde(default)]
    pub rated: bool,
    pub end_time: i64,
    #[serde(default)]
    pub eco: Option<String>,
    pub white: Side,
    pub black: Side,
}

#[derive(Deserialize)]
struct ArchivesResponse {
    archives: Vec<String>,
}

#[derive(Deserialize)]
struct GamesResponse {
    games: Vec<Game>,
}

pub enum ArchiveFetch {
    /// The archive has not changed since we stored its ETag.
    NotModified,
    Fetched { games: Vec<Game>, etag: Option<String> },
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
}

impl Client {
    pub fn new(contact: &str) -> Self {
        Self::with_base(contact, DEFAULT_BASE)
    }

    pub fn with_base(contact: &str, base: &str) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(format!("yvy-chess/0.1 (+{contact})"))
            .timeout(Duration::from_secs(90))
            .build()
            .expect("reqwest client");
        Client {
            http,
            base: base.trim_end_matches('/').to_string(),
        }
    }

    async fn get(&self, url: &str, etag: Option<&str>) -> Result<reqwest::Response, ClientError> {
        let mut attempt = 0u32;
        loop {
            attempt += 1;
            let mut req = self.http.get(url).header(header::ACCEPT, "application/json");
            if let Some(tag) = etag {
                req = req.header(header::IF_NONE_MATCH, tag);
            }
            let resp = req.send().await?;
            let status = resp.status();
            let throttled = status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
            if throttled && attempt < 5 {
                let retry_after = resp
                    .headers()
                    .get(header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok());
                let wait = retry_after.unwrap_or(2u64.pow(attempt)).min(60);
                tracing::warn!(url, status = status.as_u16(), wait, "chess.com throttled us, retrying");
                tokio::time::sleep(Duration::from_secs(wait)).await;
                continue;
            }
            return Ok(resp);
        }
    }

    /// `None` when chess.com does not know the user.
    pub async fn player(&self, username: &str) -> Result<Option<Player>, ClientError> {
        let url = format!("{}/player/{username}", self.base);
        let resp = self.get(&url, None).await?;
        match resp.status() {
            StatusCode::NOT_FOUND => Ok(None),
            s if s.is_success() => Ok(Some(resp.json().await?)),
            s => Err(ClientError::Status { status: s.as_u16(), url }),
        }
    }

    pub async fn stats(&self, username: &str) -> Result<Value, ClientError> {
        let url = format!("{}/player/{username}/stats", self.base);
        let resp = self.get(&url, None).await?;
        match resp.status() {
            StatusCode::NOT_FOUND => Ok(Value::Object(Default::default())),
            s if s.is_success() => Ok(resp.json().await?),
            s => Err(ClientError::Status { status: s.as_u16(), url }),
        }
    }

    /// Monthly archive URLs, oldest first.
    pub async fn archives(&self, username: &str) -> Result<Vec<String>, ClientError> {
        let url = format!("{}/player/{username}/games/archives", self.base);
        let resp = self.get(&url, None).await?;
        match resp.status() {
            StatusCode::NOT_FOUND => Ok(Vec::new()),
            s if s.is_success() => Ok(resp.json::<ArchivesResponse>().await?.archives),
            s => Err(ClientError::Status { status: s.as_u16(), url }),
        }
    }

    pub async fn archive(&self, url: &str, etag: Option<&str>) -> Result<ArchiveFetch, ClientError> {
        let resp = self.get(url, etag).await?;
        match resp.status() {
            StatusCode::NOT_MODIFIED => Ok(ArchiveFetch::NotModified),
            s if s.is_success() => {
                let etag = resp
                    .headers()
                    .get(header::ETAG)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());
                let body: GamesResponse = resp.json().await?;
                Ok(ArchiveFetch::Fetched { games: body.games, etag })
            }
            s => Err(ClientError::Status { status: s.as_u16(), url: url.to_string() }),
        }
    }
}

/// Extracts (year, month) from an archive URL ending in `/games/YYYY/MM`.
pub fn parse_archive_month(url: &str) -> Option<(i32, i32)> {
    let mut parts = url.trim_end_matches('/').rsplit('/');
    let month: i32 = parts.next()?.parse().ok()?;
    let year: i32 = parts.next()?.parse().ok()?;
    if (1..=12).contains(&month) && year >= 2000 {
        Some((year, month))
    } else {
        None
    }
}

/// Reduces the `/stats` payload to `{"bullet": 1500, "blitz": ..., "rapid": ..., "daily": ...}`.
pub fn ratings_from_stats(stats: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for (key, name) in [
        ("chess_bullet", "bullet"),
        ("chess_blitz", "blitz"),
        ("chess_rapid", "rapid"),
        ("chess_daily", "daily"),
    ] {
        if let Some(rating) = stats
            .get(key)
            .and_then(|v| v.get("last"))
            .and_then(|v| v.get("rating"))
            .and_then(|v| v.as_i64())
        {
            out.insert(name.to_string(), Value::from(rating));
        }
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_archive_months() {
        assert_eq!(parse_archive_month("https://api.chess.com/pub/player/x/games/2025/01"), Some((2025, 1)));
        assert_eq!(parse_archive_month("https://api.chess.com/pub/player/x/games/2025/13"), None);
        assert_eq!(parse_archive_month("nonsense"), None);
    }

    #[test]
    fn extracts_ratings() {
        let stats = serde_json::json!({
            "chess_blitz": {"last": {"rating": 1500, "rd": 30}},
            "chess_rapid": {"last": {"rating": 1600}},
            "tactics": {"highest": {"rating": 2000}}
        });
        assert_eq!(ratings_from_stats(&stats), serde_json::json!({"blitz": 1500, "rapid": 1600}));
    }
}

use serde::{Deserialize, Serialize};
use yvy_core::PlayerMove;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AppError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Internal(String),
}

pub type AppResult<T> = Result<T, AppError>;

/// Restricts which games count. Applied at query time, so changing it needs no re-indexing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Filters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_classes: Option<Vec<String>>,
    /// Earliest game end, in milliseconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_from: Option<i64>,
    /// Latest game end, in milliseconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_to: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rated: Option<bool>,
}

impl Filters {
    pub fn normalized(mut self) -> Self {
        self.time_classes = self.time_classes.and_then(|v| {
            let v: Vec<String> = v
                .into_iter()
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect();
            (!v.is_empty()).then_some(v)
        });
        self
    }

    pub fn allows(&self, game: &GameMeta) -> bool {
        if let Some(classes) = &self.time_classes {
            if !classes.iter().any(|c| c == &game.time_class) {
                return false;
            }
        }
        if self.date_from.is_some_and(|from| game.end_time < from) {
            return false;
        }
        if self.date_to.is_some_and(|to| game.end_time > to) {
            return false;
        }
        if self.rated.is_some_and(|rated| game.rated != rated) {
            return false;
        }
        true
    }
}

/// What we keep about one game. `end_time` is in milliseconds since the Unix epoch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameMeta {
    /// The chess.com game URL, or `yvy:session:<id>` for a game played in the app.
    pub id: String,
    /// The player's color in this game.
    pub color: String,
    /// bullet | blitz | rapid | daily, or `yvy` for games played in the app.
    pub time_class: String,
    pub rated: bool,
    pub end_time: i64,
    /// Raw chess.com result code for the player: win, resigned, timeout, agreed, ...
    pub result: String,
    pub opponent: Option<String>,
    pub player_rating: Option<i32>,
    pub opponent_rating: Option<i32>,
    pub eco: Option<String>,
    pub ply_count: u32,
}

/// A game and the player's decisions in it, as stored per imported month.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredGame {
    pub meta: GameMeta,
    pub decisions: Vec<PlayerMove>,
}

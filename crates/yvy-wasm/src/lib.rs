//! The browser-facing API. Structured values cross the boundary as JSON strings and month
//! data as bytes; errors become JavaScript exceptions with a readable message. Time comes from
//! the browser clock.

use serde::{de::DeserializeOwned, Serialize};
use wasm_bindgen::prelude::*;
use yvy_app::{CreateSession, EngineResult, Filters, Session, Store};

fn now() -> i64 {
    js_sys::Date::now() as i64
}

fn js_err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

fn to_json<T: Serialize>(v: &T) -> Result<String, JsError> {
    serde_json::to_string(v).map_err(js_err)
}

fn from_json<T: DeserializeOwned>(s: &str) -> Result<T, JsError> {
    serde_json::from_str(s).map_err(|e| js_err(format!("invalid input: {e}")))
}

/// One imported month: the bytes to persist and the import statistics as JSON.
#[wasm_bindgen]
pub struct ImportedMonth {
    blob: Vec<u8>,
    stats: String,
}

#[wasm_bindgen]
impl ImportedMonth {
    #[wasm_bindgen(getter)]
    pub fn blob(&self) -> Vec<u8> {
        self.blob.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn stats(&self) -> String {
        self.stats.clone()
    }
}

#[wasm_bindgen]
pub struct App {
    store: Store,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl App {
    #[wasm_bindgen(constructor)]
    pub fn new() -> App {
        console_error_panic_hook::set_once();
        App { store: Store::new() }
    }

    /// Indexes one month of `/pub/player/{u}/games/{YYYY}/{MM}` JSON.
    pub fn import_month(&mut self, username: &str, json: &str) -> Result<ImportedMonth, JsError> {
        let mut imported = self.store.import_month(username, json).map_err(js_err)?;
        let blob = std::mem::take(&mut imported.blob);
        Ok(ImportedMonth { blob, stats: to_json(&imported)? })
    }

    /// Loads a month persisted from [`App::import_month`]; returns how many games were new.
    pub fn load_month(&mut self, username: &str, blob: &[u8]) -> Result<u32, JsError> {
        self.store.load_month(username, blob).map(|n| n as u32).map_err(js_err)
    }

    pub fn finish_loading(&mut self, username: &str) {
        self.store.finish_loading(username);
    }

    pub fn has_player(&self, username: &str) -> bool {
        self.store.has_player(username)
    }

    pub fn forget_player(&mut self, username: &str) {
        self.store.forget_player(username);
    }

    pub fn player_summary(&mut self, username: &str) -> Result<String, JsError> {
        to_json(&self.store.player_summary(username))
    }

    pub fn position(&mut self, username: &str, fen: &str, filters_json: &str) -> Result<String, JsError> {
        let filters: Filters = from_json(filters_json)?;
        to_json(&self.store.position(username, fen, &filters.normalized()).map_err(js_err)?)
    }

    pub fn load_session(&mut self, json: &str) -> Result<(), JsError> {
        let session: Session = from_json(json)?;
        self.store.load_session(session).map_err(js_err)
    }

    pub fn has_session(&self, id: &str) -> bool {
        self.store.has_session(id)
    }

    /// The session exactly as it should be persisted.
    pub fn session_json(&self, id: &str) -> Result<String, JsError> {
        to_json(self.store.session(id).map_err(js_err)?)
    }

    pub fn session_username(&self, id: &str) -> Result<String, JsError> {
        Ok(self.store.session(id).map_err(js_err)?.username.clone())
    }

    pub fn sessions_for(&self, username: &str) -> Result<String, JsError> {
        to_json(&self.store.sessions_for(username))
    }

    pub fn create_session(&mut self, id: &str, request_json: &str) -> Result<String, JsError> {
        let req: CreateSession = from_json(request_json)?;
        to_json(&self.store.create_session(id, req, now()).map_err(js_err)?)
    }

    pub fn play(&mut self, id: &str, uci: &str) -> Result<String, JsError> {
        to_json(&self.store.play(id, uci, now()).map_err(js_err)?)
    }

    pub fn engine_result(&mut self, id: &str, result_json: &str) -> Result<String, JsError> {
        let result: EngineResult = from_json(result_json)?;
        to_json(&self.store.engine_result(id, &result, now()).map_err(js_err)?)
    }

    pub fn resume(&mut self, id: &str) -> Result<String, JsError> {
        to_json(&self.store.resume(id, now()).map_err(js_err)?)
    }

    pub fn undo(&mut self, id: &str) -> Result<String, JsError> {
        to_json(&self.store.undo(id, now()).map_err(js_err)?)
    }

    pub fn resign(&mut self, id: &str) -> Result<String, JsError> {
        to_json(&self.store.resign(id, now()).map_err(js_err)?)
    }

    pub fn state(&mut self, id: &str) -> Result<String, JsError> {
        to_json(&self.store.state(id).map_err(js_err)?)
    }

    pub fn pgn(&self, id: &str) -> Result<String, JsError> {
        self.store.pgn(id).map_err(js_err)
    }
}

use std::{collections::HashMap, sync::Arc};

use sqlx::PgPool;
use tokio::sync::{watch, Mutex};

use crate::{chesscom, config::Config, engine::EngineHandle, import::Progress};

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub config: Arc<Config>,
    pub chesscom: Arc<chesscom::Client>,
    /// Import jobs by username. The receiver stays after a job finishes so its final
    /// progress can still be read.
    pub imports: Arc<Mutex<HashMap<String, watch::Receiver<Progress>>>>,
    pub engine: Arc<EngineHandle>,
    /// Serializes move making. This is a single-user app; one lock is plenty.
    pub play_lock: Arc<Mutex<()>>,
}

impl AppState {
    pub fn new(pool: PgPool, config: Config) -> Self {
        let chesscom = Arc::new(chesscom::Client::new(&config.contact));
        let engine = Arc::new(EngineHandle::new(
            config.stockfish_path.clone(),
            config.engine_movetime_ms,
        ));
        AppState {
            pool,
            config: Arc::new(config),
            chesscom,
            imports: Arc::new(Mutex::new(HashMap::new())),
            engine,
            play_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn for_tests(pool: PgPool) -> Self {
        Self::new(pool, Config::for_tests())
    }
}

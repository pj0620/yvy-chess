use std::convert::Infallible;

use axum::{
    extract::{Path, Query, State},
    http::{header, StatusCode},
    middleware,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use futures::Stream;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::watch;
use tower_http::{
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use uuid::Uuid;

use crate::{
    auth, db,
    error::{AppError, AppResult},
    import::{self, ImportOptions, JobStatus, Progress},
    model::{self, Filters},
    session::{self, CreateSession},
    state::AppState,
};

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .route("/engine", get(engine_info))
        .route("/players/{username}", get(player_summary))
        .route("/players/{username}/import", post(start_import).get(import_status))
        .route("/players/{username}/import/events", get(import_events))
        .route("/players/{username}/positions", get(positions))
        .route("/players/{username}/sessions", get(player_sessions))
        .route("/sessions", post(create_session))
        .route("/sessions/{id}", get(get_session))
        .route("/sessions/{id}/moves", post(play_move))
        .route("/sessions/{id}/undo", post(undo))
        .route("/sessions/{id}/resign", post(resign))
        .route("/sessions/{id}/pgn", get(session_pgn));

    let mut app = Router::new().nest("/api", api);
    if let Some(dir) = &state.config.web_dir {
        // `fallback` (not `not_found_service`) keeps the 200 status, so deep links like
        // /g/<id> load the SPA instead of a 404 page.
        let index = ServeFile::new(dir.join("index.html"));
        app = app.fallback_service(ServeDir::new(dir).fallback(index));
    }
    app.layer(middleware::from_fn_with_state(state.clone(), auth::require_password))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> AppResult<Json<Value>> {
    sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&state.pool).await?;
    let engine = match state.engine.info().await {
        Ok(_) => "ok",
        Err(_) => "unavailable",
    };
    Ok(Json(json!({ "ok": true, "db": "ok", "engine": engine })))
}

async fn engine_info(State(state): State<AppState>) -> Json<Value> {
    match state.engine.info().await {
        Ok(info) => Json(json!({
            "available": true,
            "name": info.name,
            "path": info.path,
            "elo_min": info.elo_min,
            "elo_max": info.elo_max,
            "movetime_ms": state.engine.movetime_ms(),
        })),
        Err(e) => Json(json!({
            "available": false,
            "path": state.config.stockfish_path,
            "error": e.to_string(),
            "hint": "install Stockfish (brew install stockfish) or set STOCKFISH_PATH",
        })),
    }
}

async fn player_summary(State(state): State<AppState>, Path(username): Path<String>) -> AppResult<Json<Value>> {
    let username = db::normalize_username(&username);
    let import = state.imports.lock().await.get(&username).map(|rx| rx.borrow().clone());
    let Some(player) = db::get_player(&state.pool, &username).await? else {
        if import.is_some() {
            return Ok(Json(json!({ "username": username, "imported": false, "import": import })));
        }
        return Err(AppError::NotFound(format!("{username} has not been imported")));
    };
    let counts = db::game_counts(&state.pool, &username).await?;
    Ok(Json(json!({
        "username": player.username,
        "imported": true,
        "player_id": player.player_id,
        "joined": player.joined,
        "last_import_at": player.last_import_at,
        "ratings": player.ratings.0,
        "games": counts,
        "import": import,
    })))
}

#[derive(Deserialize, Default)]
struct SessionsQuery {
    limit: Option<i64>,
}

/// Games played in the app, most recently touched first.
async fn player_sessions(
    State(state): State<AppState>,
    Path(username): Path<String>,
    Query(q): Query<SessionsQuery>,
) -> AppResult<Json<Vec<db::SessionSummary>>> {
    let username = db::normalize_username(&username);
    let limit = q.limit.unwrap_or(30).clamp(1, 200);
    Ok(Json(db::list_sessions(&state.pool, &username, limit).await?))
}

#[derive(Deserialize, Default)]
struct ImportQuery {
    months: Option<u32>,
}

async fn start_import(
    State(state): State<AppState>,
    Path(username): Path<String>,
    Query(q): Query<ImportQuery>,
) -> AppResult<(StatusCode, Json<Value>)> {
    let username = db::normalize_username(&username);
    if username.is_empty() || !username.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(AppError::BadRequest("that does not look like a chess.com username".to_string()));
    }
    if let Some(rx) = state.imports.lock().await.get(&username) {
        let progress = rx.borrow().clone();
        if progress.status == JobStatus::Running {
            return Ok((StatusCode::OK, Json(json!({ "status": "already_running", "progress": progress }))));
        }
    }
    state
        .chesscom
        .player(&username)
        .await
        .map_err(|e| AppError::Upstream(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("chess.com user \"{username}\" was not found")))?;
    let rx = import::spawn(state.clone(), username, ImportOptions { months: q.months }).await;
    let progress = rx.borrow().clone();
    Ok((StatusCode::ACCEPTED, Json(json!({ "status": "started", "progress": progress }))))
}

async fn import_status(State(state): State<AppState>, Path(username): Path<String>) -> AppResult<Json<Progress>> {
    let username = db::normalize_username(&username);
    let progress = state.imports.lock().await.get(&username).map(|rx| rx.borrow().clone());
    progress
        .map(Json)
        .ok_or_else(|| AppError::NotFound(format!("no import has been started for {username}")))
}

enum SseStep {
    Initial,
    Waiting,
    Done,
}

async fn import_events(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> AppResult<Sse<impl Stream<Item = Result<Event, Infallible>>>> {
    let username = db::normalize_username(&username);
    let rx = state
        .imports
        .lock()
        .await
        .get(&username)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("no import has been started for {username}")))?;

    fn emit(rx: &mut watch::Receiver<Progress>) -> (Event, SseStep) {
        let progress = rx.borrow_and_update().clone();
        let next = if progress.status == JobStatus::Running {
            SseStep::Waiting
        } else {
            SseStep::Done
        };
        let event = Event::default()
            .json_data(&progress)
            .unwrap_or_else(|_| Event::default().data("{}"));
        (event, next)
    }

    let stream = futures::stream::unfold((rx, SseStep::Initial), |(mut rx, step)| async move {
        match step {
            SseStep::Done => None,
            SseStep::Initial => {
                let (event, next) = emit(&mut rx);
                Some((Ok(event), (rx, next)))
            }
            SseStep::Waiting => {
                if rx.changed().await.is_err() {
                    return None;
                }
                let (event, next) = emit(&mut rx);
                Some((Ok(event), (rx, next)))
            }
        }
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

#[derive(Deserialize)]
struct PositionsQuery {
    epd: Option<String>,
    fen: Option<String>,
    time_classes: Option<String>,
    date_from: Option<DateTime<Utc>>,
    date_to: Option<DateTime<Utc>>,
    rated: Option<bool>,
}

async fn positions(
    State(state): State<AppState>,
    Path(username): Path<String>,
    Query(q): Query<PositionsQuery>,
) -> AppResult<Json<Value>> {
    let username = db::normalize_username(&username);
    let epd = match (&q.epd, &q.fen) {
        (Some(epd), _) => epd.trim().to_string(),
        (None, Some(fen)) => model::key_from_fen(fen).map_err(AppError::BadRequest)?,
        (None, None) => return Err(AppError::BadRequest("pass ?epd=... or ?fen=...".to_string())),
    };
    let filters = Filters {
        time_classes: q
            .time_classes
            .as_ref()
            .map(|s| s.split(',').map(|x| x.to_string()).collect()),
        date_from: q.date_from,
        date_to: q.date_to,
        rated: q.rated,
    }
    .normalized();
    let candidates = model::candidates(&state.pool, &username, &epd, &filters).await?;
    Ok(Json(json!({
        "username": username,
        "epd": epd,
        "filters": filters,
        "games": model::total_games(&candidates),
        "candidates": candidates,
    })))
}

async fn create_session(
    State(state): State<AppState>,
    Json(req): Json<CreateSession>,
) -> AppResult<(StatusCode, Json<session::SessionState>)> {
    let s = session::create(&state, req).await?;
    Ok((StatusCode::CREATED, Json(s)))
}

async fn get_session(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<session::SessionState>> {
    Ok(Json(session::get(&state, id).await?))
}

#[derive(Deserialize)]
struct MoveBody {
    uci: String,
}

async fn play_move(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<MoveBody>,
) -> AppResult<Json<session::SessionState>> {
    Ok(Json(session::play(&state, id, &body.uci).await?))
}

async fn undo(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<session::SessionState>> {
    Ok(Json(session::undo(&state, id).await?))
}

async fn resign(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<session::SessionState>> {
    Ok(Json(session::resign(&state, id).await?))
}

async fn session_pgn(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<impl IntoResponse> {
    let s = session::get(&state, id).await?;
    let pgn = session::to_pgn(&s);
    Ok((
        [
            (header::CONTENT_TYPE, "application/x-chess-pgn; charset=utf-8".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"yvy-{}.pgn\"", s.id)),
        ],
        pgn,
    ))
}

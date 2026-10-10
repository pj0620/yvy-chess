//! End-to-end tests through the HTTP router: JSON shapes, status codes and the password gate.

use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
    Router,
};
use base64::Engine;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use yvy_server::{api, chesscom::Game, config::Config, db, import, state::AppState};

fn fixture_games() -> Vec<Game> {
    let v: Value = serde_json::from_str(include_str!("fixtures/alice_games.json")).unwrap();
    serde_json::from_value(v["games"].clone()).unwrap()
}

async fn seed_alice(pool: &PgPool) {
    db::upsert_player(pool, "alice", Some(42), None, json!({"blitz": 1500, "rapid": 1550})).await.unwrap();
    import::index_games(pool, "alice", &fixture_games()).await.unwrap();
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn post_json(uri: &str, body: Value) -> Request<Body> {
    Request::post(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn plays_a_game_over_http(pool: PgPool) {
    seed_alice(&pool).await;
    let app = api::router(AppState::for_tests(pool));

    let (status, body) = send(&app, Request::get("/api/health").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!((body["ok"].as_bool(), body["db"].as_str(), body["engine"].as_str()), (Some(true), Some("ok"), Some("unavailable")));

    let (status, body) = send(&app, Request::get("/api/players/nobody").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body["error"].as_str().unwrap().contains("nobody"));

    let (status, summary) = send(&app, Request::get("/api/players/Alice").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(summary["games"]["total"], 4);

    let (status, s) = send(
        &app,
        post_json("/api/sessions", json!({"username": "Alice", "user_color": "black", "mode": "most_frequent", "steer_cp": 0})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{s}");
    assert_eq!((s["moves"][0]["uci"].as_str(), s["moves"][0]["source"].as_str()), (Some("e2e4"), Some("history")));
    assert!(s["moves"][0]["detail"]["candidates"][0]["last_played"].is_string());
    let id = s["id"].as_str().unwrap().to_string();

    let (status, s) = send(&app, post_json(&format!("/api/sessions/{id}/moves"), json!({"uci": "e7e5"}))).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(s["moves"][2]["uci"], "g1f3");
    assert_eq!(s["ply"], 3);

    let (status, err) = send(&app, post_json(&format!("/api/sessions/{id}/moves"), json!({"uci": "zzzz"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(err["error"].as_str().unwrap().contains("illegal"));

    let (status, list) = send(&app, Request::get("/api/players/alice/sessions").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().map(Vec::len), Some(1));
    assert_eq!((list[0]["plies"].as_i64(), list[0]["status"].as_str(), list[0]["gap_mode"].as_str()), (Some(3), Some("playing"), Some("stockfish")));

    let (status, err) = send(&app, post_json("/api/sessions", json!({"username": "alice", "user_color": "white", "gap_mode": "maia"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(err["error"].as_str().unwrap().contains("gap_mode"));

    let resp = app
        .clone()
        .oneshot(Request::get(format!("/api/sessions/{id}/pgn")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let text = String::from_utf8(to_bytes(resp.into_body(), 1 << 20).await.unwrap().to_vec()).unwrap();
    assert!(text.contains("1. e4 {you: 2/3} e5 2. Nf3 {you: 1/1} *"), "{text}");

    let (status, _) = send(&app, Request::get("/api/sessions/00000000-0000-0000-0000-000000000000").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "./migrations")]
async fn password_gate_protects_everything_but_health(pool: PgPool) {
    let mut config = Config::for_tests();
    config.password = Some("hunter2".to_string());
    let app = api::router(AppState::new(pool, config));

    let (status, _) = send(&app, Request::get("/api/health").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::OK, "platform health checks must not need the password");

    let resp = app.clone().oneshot(Request::get("/api/engine").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert!(resp.headers().get(header::WWW_AUTHENTICATE).unwrap().to_str().unwrap().starts_with("Basic"));

    let basic = |creds: &str| format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(creds));
    let resp = app
        .clone()
        .oneshot(Request::get("/api/engine").header(header::AUTHORIZATION, basic("me:wrong")).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    let (status, body) = send(
        &app,
        Request::get("/api/engine").header(header::AUTHORIZATION, basic("anyone:hunter2")).body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["available"], false);
}

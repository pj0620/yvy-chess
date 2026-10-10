//! Integration tests against the Compose Postgres. `#[sqlx::test]` creates a fresh,
//! migrated database per test using DATABASE_URL.

use serde_json::json;
use sqlx::PgPool;
use yvy_core::{GameState, Mode};
use yvy_server::{
    chesscom::Game,
    db, import,
    model::{self, Filters},
    session::{self, CreateSession},
    state::AppState,
};

const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq -";
const AFTER_E4: &str = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq -";

fn fixture_games() -> Vec<Game> {
    let v: serde_json::Value = serde_json::from_str(include_str!("fixtures/alice_games.json")).unwrap();
    serde_json::from_value(v["games"].clone()).unwrap()
}

async fn seed_alice(pool: &PgPool) -> import::IndexStats {
    db::upsert_player(pool, "alice", Some(42), None, json!({"blitz": 1500, "rapid": 1550}))
        .await
        .unwrap();
    import::index_games(pool, "alice", &fixture_games()).await.unwrap()
}

#[sqlx::test(migrations = "./migrations")]
async fn indexes_games_and_answers_lookups(pool: PgPool) {
    let stats = seed_alice(&pool).await;
    assert_eq!(stats.seen, 6);
    assert_eq!(stats.new, 4, "four standard games with valid PGNs");
    assert_eq!(stats.skipped, 2, "the chess960 game and the broken PGN");
    assert_eq!(stats.errors.len(), 1);
    assert!(stats.errors[0].contains("illegal move"));
    // Alice made 3 + 2 + 2 + 3 = 10 decisions across the four games.
    assert_eq!(stats.positions, 10);

    let start = model::candidates(&pool, "alice", START, &Filters::default()).await.unwrap();
    let summary: Vec<(&str, i64)> = start.iter().map(|c| (c.uci.as_str(), c.count)).collect();
    assert_eq!(summary, [("e2e4", 2), ("d2d4", 1)]);
    assert_eq!(start[0].san, "e4");
    assert_eq!(start[0].game_ids.len(), 2);

    let rapid_only = Filters {
        time_classes: Some(vec!["rapid".into()]),
        ..Default::default()
    };
    let start_rapid = model::candidates(&pool, "alice", START, &rapid_only).await.unwrap();
    assert_eq!(start_rapid.len(), 1);
    assert_eq!((start_rapid[0].uci.as_str(), start_rapid[0].count), ("e2e4", 1));

    let unrated = Filters {
        rated: Some(false),
        ..Default::default()
    };
    let start_unrated = model::candidates(&pool, "alice", START, &unrated).await.unwrap();
    assert_eq!((start_unrated[0].uci.as_str(), start_unrated[0].count), ("d2d4", 1));

    // As Black, Alice answered 1. e4 with 1... e5 once.
    let black = model::candidates(&pool, "alice", AFTER_E4, &Filters::default()).await.unwrap();
    assert_eq!(black.len(), 1);
    assert_eq!((black[0].uci.as_str(), black[0].count), ("e7e5", 1));

    // Re-importing the same archive is a no-op.
    let again = import::index_games(&pool, "alice", &fixture_games()).await.unwrap();
    assert_eq!(again.new, 0);
    let counts = db::game_counts(&pool, "alice").await.unwrap();
    assert_eq!((counts.total, counts.white, counts.black, counts.positions), (4, 3, 1, 10));
    assert_eq!(counts.by_time_class.get("blitz"), Some(&3));
}

#[sqlx::test(migrations = "./migrations")]
async fn plays_from_history_then_falls_back_to_the_engine(pool: PgPool) {
    seed_alice(&pool).await;
    let state = AppState::for_tests(pool);

    // Alice plays Black, so the model (White) moves first: its most frequent first move is 1. e4.
    let s = session::create(
        &state,
        CreateSession {
            username: "Alice".into(),
            user_color: "black".into(),
            engine_elo: None,
            mode: Some(Mode::MostFrequent),
            filters: None,
            steer_cp: None,
            remember: None,
            gap_mode: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(s.engine_elo, 1550, "defaults to the rapid rating");
    assert_eq!(s.moves.len(), 1);
    assert_eq!((s.moves[0].source.as_str(), s.moves[0].uci.as_str()), ("history", "e2e4"));
    assert_eq!(s.moves[0].detail.as_ref().unwrap()["games"], 3);
    assert_eq!(s.turn, "black");
    assert!(s.coverage.in_history, "Alice has faced 1. e4 as Black");
    assert_eq!(s.coverage.candidates[0].uci, "e7e5");

    // 1... e5: Alice (as White) answered that with 2. Nf3 in game 1, so the model does too.
    let s = session::play(&state, s.id, "e7e5").await.unwrap();
    assert_eq!(s.moves.len(), 3);
    assert_eq!((s.moves[2].source.as_str(), s.moves[2].uci.as_str()), ("history", "g1f3"));

    // 2... Nf6 is a position Alice never had as White, and there is no Stockfish in tests,
    // so the gap is filled with a random legal move tagged as such.
    let s = session::play(&state, s.id, "g8f6").await.unwrap();
    assert_eq!(s.moves.len(), 5);
    assert_eq!(s.moves[4].source, "engine");
    assert_eq!(s.moves[4].detail.as_ref().unwrap()["engine"], "random");
    assert!(!s.coverage.in_history);
    assert_eq!(s.status, "playing");

    // Not our turn / illegal input are rejected cleanly.
    assert!(session::play(&state, s.id, "zzzz").await.is_err());

    // Undo takes back our move and the reply.
    let s = session::undo(&state, s.id).await.unwrap();
    assert_eq!(s.moves.len(), 3);
    assert_eq!(s.turn, "black");

    let s = session::resign(&state, s.id).await.unwrap();
    assert_eq!((s.status.as_str(), s.result.as_deref()), ("resigned", Some("loss")));
    assert!(s.legal_moves.is_empty());
    assert!(session::play(&state, s.id, "g8f6").await.is_err());

    let pgn = session::to_pgn(&s);
    assert!(pgn.contains("1. e4 {you: 2/3} e5 2. Nf3 {you: 1/1} 1-0"), "pgn was:\n{pgn}");
}

#[sqlx::test(migrations = "./migrations")]
async fn checkmate_ends_the_session(pool: PgPool) {
    seed_alice(&pool).await;
    let state = AppState::for_tests(pool);
    // Alice plays White. Her history says 1. e4; Black replies come from the random engine
    // fallback, so we only check that the session state machine handles the end of a game.
    let s = session::create(
        &state,
        CreateSession {
            username: "alice".into(),
            user_color: "white".into(),
            engine_elo: Some(1400),
            mode: None,
            filters: Some(Filters {
                time_classes: Some(vec!["blitz".into(), "rapid".into()]),
                ..Default::default()
            }),
            steer_cp: None,
            remember: None,
            gap_mode: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(s.moves.len(), 0);
    assert_eq!(s.turn, "white");
    assert_eq!(s.coverage.games, 3);
    assert_eq!(s.filters.time_classes.as_deref(), Some(&["blitz".to_string(), "rapid".to_string()][..]));
    assert!(session::undo(&state, s.id).await.is_err(), "nothing to undo yet");

    // Play until the game ends or 60 of our moves have passed; the random opponent usually
    // walks into something. Whatever happens, the state must stay consistent.
    let mut s = s;
    for _ in 0..60 {
        if s.status != "playing" {
            break;
        }
        let mv = s.legal_moves[0].clone();
        s = session::play(&state, s.id, &mv).await.unwrap();
        assert_eq!(s.turn, if s.status == "playing" { "white" } else { s.turn.as_str() });
    }
    let reloaded = session::get(&state, s.id).await.unwrap();
    assert_eq!(reloaded.moves.len(), s.moves.len());
    assert_eq!(reloaded.status, s.status);
    if s.status != "playing" {
        assert!(s.result.is_some());
    }
}

#[sqlx::test(migrations = "./migrations")]
async fn steering_finds_transpositions_back_into_history(pool: PgPool) {
    seed_alice(&pool).await;
    // Alice's only Black game went 1. e4 e5 2. Nf3 Nc6 3. Bc4 Bc5. After 1. Nf3 she has no
    // history, but 1... e5 lets 2. e4 transpose straight into it.
    let mut g = GameState::new();
    g.apply_uci("g1f3").unwrap();
    let cands = model::steering_candidates(&pool, "alice", g.position(), &Filters::default()).await.unwrap();
    assert_eq!(cands[0].uci, "e7e5", "{cands:?}");
    assert_eq!((cands[0].games_ahead, cands[0].known_replies, cands[0].setup_games), (1, 1, 1));
    assert!(cands.iter().all(|c| c.games_ahead > 0 || c.setup_games > 0));

    // From the human's side after 1. Nf3 e5, only 2. e4 lands in Alice's games.
    g.apply_uci("e7e5").unwrap();
    let back = model::returning_moves(&pool, "alice", g.position(), &Filters::default()).await.unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!((back[0].uci.as_str(), back[0].san.as_str(), back[0].games), ("e2e4", "e4", 1));

    // A filter that excludes the Black game removes the path.
    let rapid = Filters { time_classes: Some(vec!["rapid".into()]), ..Default::default() };
    assert!(model::returning_moves(&pool, "alice", g.position(), &rapid).await.unwrap().is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn gap_filler_steers_back_even_without_an_engine(pool: PgPool) {
    seed_alice(&pool).await;
    let state = AppState::for_tests(pool);
    let s = session::create(
        &state,
        CreateSession {
            username: "alice".into(),
            user_color: "white".into(),
            engine_elo: Some(1500),
            mode: None,
            filters: None,
            steer_cp: None,
            remember: None,
            gap_mode: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(s.steer_cp, 100, "steering is on by default");

    // 1. Nf3 is a gap for the model (Alice as Black). Steering picks 1... e5 because 2. e4
    // would transpose into her games; without an engine that is the only input.
    let s = session::play(&state, s.id, "g1f3").await.unwrap();
    let reply = &s.moves[1];
    assert_eq!((reply.source.as_str(), reply.uci.as_str()), ("engine", "e7e5"));
    let d = reply.detail.as_ref().unwrap();
    assert_eq!(d["engine"], "random");
    assert_eq!(d["steer"]["games_ahead"], 1);
    assert_eq!(d["steer"]["considered"][0]["uci"], "e7e5");

    // And the human is told which move brings the model back: 2. e4.
    assert_eq!(s.coverage.returning_moves.len(), 1);
    assert_eq!(s.coverage.returning_moves[0].uci, "e2e4");

    // With steering off, the fallback carries no steering detail at all.
    let off = session::create(
        &state,
        CreateSession {
            username: "alice".into(),
            user_color: "white".into(),
            engine_elo: Some(1500),
            mode: None,
            filters: None,
            steer_cp: Some(0),
            remember: None,
            gap_mode: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(off.steer_cp, 0);
    let off = session::play(&state, off.id, "g1f3").await.unwrap();
    assert!(off.moves[1].detail.as_ref().unwrap().get("steer").is_none());
}

#[sqlx::test(migrations = "./migrations")]
async fn familiar_setups_steer_when_no_transposition_exists(pool: PgPool) {
    seed_alice(&pool).await;
    // 1. e4 e5 2. Bc4: Alice (Black) never faced 2. Bc4, and no reply transposes into her games
    // except via 2... Nc6 3. Nf3. Her other Black setups still count: after 2... Nc6 her pieces
    // stand exactly as after 2. Nf3 Nc6 in game 4.
    let mut g = GameState::new();
    for m in ["e2e4", "e7e5", "f1c4"] {
        g.apply_uci(m).unwrap();
    }
    let cands = model::steering_candidates(&pool, "alice", g.position(), &Filters::default()).await.unwrap();
    let nc6 = cands.iter().find(|c| c.uci == "b8c6").expect("Nc6 is a candidate");
    assert_eq!((nc6.games_ahead, nc6.setup_games), (1, 1));
    assert_eq!(cands[0].uci, "b8c6", "exact transposition ranks first: {cands:?}");

    // Reindexing from the stored PGNs is idempotent and fills the setup column.
    let rows = import::reindex(&pool, "alice").await.unwrap();
    assert_eq!(rows, 10);
    let missing: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM position_moves WHERE setup IS NULL")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(missing, 0);
    assert!(import::players_needing_reindex(&pool).await.unwrap().is_empty());
}

#[sqlx::test(migrations = "./migrations")]
async fn games_played_here_are_remembered(pool: PgPool) {
    seed_alice(&pool).await;
    let state = AppState::for_tests(pool.clone());
    let s = session::create(
        &state,
        CreateSession {
            username: "alice".into(),
            user_color: "white".into(),
            engine_elo: Some(1500),
            mode: None,
            filters: None,
            steer_cp: Some(0),
            remember: None,
            gap_mode: None,
        },
    )
    .await
    .unwrap();
    assert!(s.remember, "remembering is on by default");
    let game_id = session::session_game_id(s.id);

    // Before any move, the start position shows Alice's two e4 games and one d4 game.
    let before = model::candidates(&pool, "alice", START, &Filters::default()).await.unwrap();
    assert_eq!(before.iter().find(|c| c.uci == "d2d4").map(|c| c.count), Some(1));

    // Her move in this game is added at once, as a game with time class yvy.
    let s = session::play(&state, s.id, "d2d4").await.unwrap();
    let after = model::candidates(&pool, "alice", START, &Filters::default()).await.unwrap();
    let d4 = after.iter().find(|c| c.uci == "d2d4").unwrap();
    assert_eq!(d4.count, 2);
    assert!(d4.game_ids.contains(&game_id));
    let (class, result, plies): (String, String, i32) =
        sqlx::query_as("SELECT time_class, result, ply_count FROM games WHERE id = $1")
            .bind(&game_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((class.as_str(), result.as_str(), plies), ("yvy", "playing", 2));
    // Only Alice's move is recorded, not the model's reply.
    let rows: Vec<(i32, String)> = sqlx::query_as("SELECT ply, uci FROM position_moves WHERE game_id = $1 ORDER BY ply")
        .bind(&game_id)
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(rows, vec![(1, "d2d4".to_string())]);
    let setup: Option<String> = sqlx::query_scalar("SELECT setup FROM position_moves WHERE game_id = $1 AND ply = 1")
        .bind(&game_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(setup.as_deref(), Some("8/8/8/8/3P4/8/PPP1PPPP/RNBQKBNR"));

    // Excluding app games by time class hides it again.
    let imported_only = Filters { time_classes: Some(vec!["blitz".into(), "rapid".into()]), ..Default::default() };
    let filtered = model::candidates(&pool, "alice", START, &imported_only).await.unwrap();
    assert_eq!(filtered.iter().find(|c| c.uci == "d2d4").map(|c| c.count), Some(1));

    // Undo removes the memory; a different move replaces it.
    session::undo(&state, s.id).await.unwrap();
    let undone = model::candidates(&pool, "alice", START, &Filters::default()).await.unwrap();
    assert_eq!(undone.iter().find(|c| c.uci == "d2d4").map(|c| c.count), Some(1));
    let s = session::play(&state, s.id, "e2e4").await.unwrap();
    let rows: Vec<(i32, String)> = sqlx::query_as("SELECT ply, uci FROM position_moves WHERE game_id = $1 ORDER BY ply")
        .bind(&game_id)
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(rows, vec![(1, "e2e4".to_string())]);

    // Resigning records the result.
    session::resign(&state, s.id).await.unwrap();
    let result: String = sqlx::query_scalar("SELECT result FROM games WHERE id = $1").bind(&game_id).fetch_one(&pool).await.unwrap();
    assert_eq!(result, "resigned");

    // A game with remembering off leaves no trace.
    let quiet = session::create(
        &state,
        CreateSession {
            username: "alice".into(),
            user_color: "white".into(),
            engine_elo: Some(1500),
            mode: None,
            filters: None,
            steer_cp: Some(0),
            remember: Some(false),
            gap_mode: None,
        },
    )
    .await
    .unwrap();
    session::play(&state, quiet.id, "d2d4").await.unwrap();
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM games WHERE id = $1)")
        .bind(session::session_game_id(quiet.id))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!exists);
}

#[sqlx::test(migrations = "./migrations")]
async fn switch_mode_hands_the_gap_to_the_human(pool: PgPool) {
    seed_alice(&pool).await;
    let state = AppState::for_tests(pool.clone());
    let s = session::create(
        &state,
        CreateSession {
            username: "alice".into(),
            user_color: "white".into(),
            engine_elo: None,
            mode: None,
            filters: None,
            steer_cp: None,
            remember: None,
            gap_mode: Some("switch".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!((s.gap_mode.as_str(), s.current_color.as_str()), ("switch", "white"));

    // Alice never faced 1. d4 as Black, so instead of an engine move the human takes over Black.
    let s = session::play(&state, s.id, "d2d4").await.unwrap();
    assert_eq!(s.moves.len(), 1, "no model move was played");
    assert_eq!((s.current_color.as_str(), s.turn.as_str()), ("black", "black"));
    assert_eq!(s.switches, vec![session::Switch { ply: 1, color: "black".into() }]);
    assert!(s.legal_moves.contains(&"d7d5".to_string()));

    // The human answers as Black. That decision is remembered as Alice's own, for Black.
    let s = session::play(&state, s.id, "d7d5").await.unwrap();
    let after_d4 = "rnbqkbnr/pppppppp/8/8/3P4/8/PPP1PPPP/RNBQKBNR b KQkq -";
    let learned = model::candidates(&pool, "alice", after_d4, &Filters::default()).await.unwrap();
    assert_eq!((learned.len(), learned[0].uci.as_str(), learned[0].count), (1, "d7d5", 1));
    // Now the model plays White from Alice's White history: 1. d4 d5 2. c4 (game 3).
    assert_eq!(s.moves.len(), 3);
    assert_eq!((s.moves[2].source.as_str(), s.moves[2].uci.as_str()), ("history", "c2c4"));
    assert_eq!(s.current_color, "black");

    // 2... e6 leaves Alice's White history too (game 3 ended there), so the human takes White back.
    let s = session::play(&state, s.id, "e7e6").await.unwrap();
    assert_eq!(s.moves.len(), 4);
    assert_eq!((s.current_color.as_str(), s.turn.as_str()), ("white", "white"));
    assert_eq!(s.switches.len(), 2);
    assert_eq!(s.switches[1], session::Switch { ply: 4, color: "white".into() });
    assert!(s.moves.iter().all(|m| m.source != "engine"), "no engine move in switch mode");

    // Undo takes back the human's e6 and restores them to Black, dropping the later switch.
    let s = session::undo(&state, s.id).await.unwrap();
    assert_eq!(s.moves.len(), 3);
    assert_eq!((s.current_color.as_str(), s.turn.as_str()), ("black", "black"));
    assert_eq!(s.switches.len(), 1);

    let pgn = session::to_pgn(&s);
    assert!(pgn.contains("1. d4 {you take over as black} d5 2. c4 {you: 1/1}"), "pgn was:\n{pgn}");

    // A session that starts as Black with no usable White history switches before any move.
    let s = session::create(
        &state,
        CreateSession {
            username: "alice".into(),
            user_color: "black".into(),
            engine_elo: None,
            mode: None,
            filters: Some(Filters { time_classes: Some(vec!["daily".into()]), ..Default::default() }),
            steer_cp: None,
            remember: Some(false),
            gap_mode: Some("switch".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(s.moves.len(), 0);
    assert_eq!((s.current_color.as_str(), s.user_color.as_str()), ("white", "black"));
    assert_eq!(s.switches, vec![session::Switch { ply: 0, color: "white".into() }]);
}

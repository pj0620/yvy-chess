//! The scenarios the old server's Postgres integration tests covered, now against the
//! in-memory store that runs in the browser.

use serde_json::json;
use yvy_app::{
    session::{self, steering_candidates},
    CreateSession, EngineLine, EngineResult, Filters, Store, Switch,
};
use yvy_core::{GameState, Mode};

const FIXTURE: &str = include_str!("fixtures/alice_games.json");
const START: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
const AFTER_E4: &str = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1";
const AFTER_D4: &str = "rnbqkbnr/pppppppp/8/8/3P4/8/PPP1PPPP/RNBQKBNR b KQkq - 0 1";

fn alice() -> Store {
    let mut st = Store::new();
    st.import_month("alice", FIXTURE).unwrap();
    st.finish_loading("alice");
    st
}

fn counts(st: &mut Store, fen: &str, f: &Filters) -> Vec<(String, i64)> {
    st.position("alice", fen, f).unwrap().into_iter().map(|c| (c.uci, c.count)).collect()
}

fn req(color: &str) -> CreateSession {
    CreateSession {
        username: "alice".into(),
        user_color: color.into(),
        ratings: Some(json!({"blitz": 1500, "rapid": 1550})),
        ..Default::default()
    }
}

fn no_engine() -> EngineResult {
    EngineResult { error: Some("no engine in tests".into()), ..Default::default() }
}

#[test]
fn indexes_games_and_answers_lookups() {
    let mut st = Store::new();
    let r = st.import_month("Alice", FIXTURE).unwrap();
    assert_eq!((r.seen, r.new, r.skipped), (6, 4, 2), "four standard games; the chess960 game and the broken PGN are skipped");
    assert_eq!(r.errors.len(), 1);
    assert!(r.errors[0].contains("illegal move"));
    assert_eq!(r.positions, 10, "Alice made 3 + 2 + 2 + 3 decisions");
    st.finish_loading("alice");

    let all = Filters::default();
    assert_eq!(counts(&mut st, START, &all), [("e2e4".to_string(), 2), ("d2d4".to_string(), 1)]);
    let start = st.position("alice", START, &all).unwrap();
    assert_eq!(start[0].san, "e4");
    assert_eq!(start[0].game_ids.len(), 2);
    assert_eq!(start[0].game_ids[0], "https://www.chess.com/game/live/2", "most recent first");
    assert_eq!(start[0].last_played, 1_700_100_000_000);

    let rapid = Filters { time_classes: Some(vec!["rapid".into()]), ..Default::default() };
    assert_eq!(counts(&mut st, START, &rapid), [("e2e4".to_string(), 1)]);
    let unrated = Filters { rated: Some(false), ..Default::default() };
    assert_eq!(counts(&mut st, START, &unrated), [("d2d4".to_string(), 1)]);
    let late = Filters { date_from: Some(1_700_150_000_000), ..Default::default() };
    assert_eq!(counts(&mut st, START, &late), [("d2d4".to_string(), 1)]);
    assert_eq!(counts(&mut st, AFTER_E4, &all), [("e7e5".to_string(), 1)], "as Black she answered 1. e4 with e5 once");

    // Importing the same month again changes nothing; the blob reloads into an equal index.
    let again = st.import_month("alice", FIXTURE).unwrap();
    assert_eq!(again.new, 0);
    let summary = st.player_summary("alice");
    assert_eq!((summary.total, summary.white, summary.black, summary.positions), (4, 3, 1, 10));
    assert_eq!(summary.by_time_class.get("blitz"), Some(&3));

    let mut fresh = Store::new();
    assert_eq!(fresh.load_month("alice", &r.blob).unwrap(), 4);
    fresh.finish_loading("alice");
    assert_eq!(fresh.player_summary("alice"), summary);
    assert_eq!(counts(&mut fresh, START, &all), counts(&mut st, START, &all));
    assert!(fresh.load_month("alice", b"not a blob").is_err());
}

#[test]
fn plays_from_history_then_asks_the_engine() {
    let mut st = alice();
    // Alice plays Black, so the model (White) moves first: its most frequent first move is e4.
    let step = st
        .create_session("s1", CreateSession { mode: Some(Mode::MostFrequent), ..req("black") }, 1000)
        .unwrap();
    let s = step.state;
    assert!(step.engine_request.is_none());
    assert_eq!(s.engine_elo, 1550, "defaults to the rapid rating");
    assert_eq!(s.moves.len(), 1);
    assert_eq!((s.moves[0].source.as_str(), s.moves[0].uci.as_str()), ("history", "e2e4"));
    assert_eq!(s.moves[0].detail.as_ref().unwrap()["games"], 3);
    assert_eq!(s.turn, "black");
    assert!(s.coverage.in_history);
    assert_eq!(s.coverage.candidates[0].uci, "e7e5");

    // 1... e5: as White she answered that with 2. Nf3 in game 1, so the model does too.
    let s = st.play("s1", "e7e5", 2000).unwrap().state;
    assert_eq!((s.moves[2].source.as_str(), s.moves[2].uci.as_str()), ("history", "g1f3"));

    // 2... Nf6 is new for White-Alice: the model needs the engine.
    let step = st.play("s1", "g8f6", 3000).unwrap();
    assert_eq!(step.state.moves.len(), 4);
    assert_eq!(step.state.turn, "white");
    let request = step.engine_request.expect("an engine request");
    assert_eq!(request.fen, step.state.fen);
    assert_eq!((request.elo, request.uci_elo, request.skill_level), (1550, Some(1550), None));

    // The engine plays its best move when steering has nothing to offer or it is the best.
    let best = step.state.legal_moves.iter().find(|m| *m == "b1c3").unwrap().clone();
    let result = EngineResult {
        fen: request.fen.clone(),
        best: Some(EngineLine { uci: best.clone(), depth: Some(12), score_cp: Some(30), score_mate: None }),
        ..Default::default()
    };
    let s = st.engine_result("s1", &result, 4000).unwrap().state;
    assert_eq!(s.moves.len(), 5);
    let reply = &s.moves[4];
    assert_eq!(reply.source, "engine");
    assert_eq!(reply.detail.as_ref().unwrap()["engine"], "stockfish");
    assert_eq!(reply.detail.as_ref().unwrap()["depth"], 12);
    // A late answer for a position the game has left is ignored.
    assert_eq!(st.engine_result("s1", &result, 4500).unwrap().state.moves.len(), 5);

    assert!(st.play("s1", "zzzz", 5000).is_err());
    let s = st.undo("s1", 6000).unwrap().state;
    assert_eq!(s.moves.len(), 3);
    assert_eq!(s.turn, "black");

    let s = st.resign("s1", 7000).unwrap().state;
    assert_eq!((s.status.as_str(), s.result.as_deref()), ("resigned", Some("loss")));
    assert!(s.legal_moves.is_empty());
    assert!(st.play("s1", "g8f6", 8000).is_err());
    assert!(st.undo("s1", 8000).is_err());

    let pgn = st.pgn("s1").unwrap();
    assert!(pgn.contains("1. e4 {you: 2/3} e5 2. Nf3 {you: 1/1} 1-0"), "pgn was:\n{pgn}");
    assert!(pgn.contains("[Date \"1970.01.01\"]"));
}

#[test]
fn sessions_survive_a_reload() {
    let mut st = alice();
    st.create_session("g", CreateSession { steer_cp: Some(0), ..req("white") }, 10).unwrap();
    let mut step = st.play("g", "d2d4", 20).unwrap();
    for _ in 0..30 {
        if let Some(r) = &step.engine_request {
            step = st.engine_result("g", &EngineResult { fen: r.fen.clone(), ..no_engine() }, 30).unwrap();
            continue;
        }
        if step.state.status != "playing" {
            break;
        }
        let mv = step.state.legal_moves[0].clone();
        step = st.play("g", &mv, 40).unwrap();
    }
    let before = st.state("g").unwrap();
    assert!(before.moves.len() > 10);
    assert!(before.moves.iter().filter(|m| m.source == "engine").all(|m| m.detail.as_ref().unwrap()["engine"] == "random"));

    // What the browser persists: the session JSON and the month blob.
    let json = serde_json::to_string(st.session("g").unwrap()).unwrap();
    let blob = Store::new().import_month("alice", FIXTURE).unwrap().blob;
    let mut reloaded = Store::new();
    reloaded.load_month("alice", &blob).unwrap();
    reloaded.finish_loading("alice");
    reloaded.load_session(serde_json::from_str(&json).unwrap()).unwrap();
    assert_eq!(reloaded.state("g").unwrap(), before);
    assert_eq!(reloaded.player_summary("alice"), st.player_summary("alice"), "the remembered game comes back too");
    assert_eq!(reloaded.sessions_for("ALICE").len(), 1);
}

#[test]
fn steering_finds_transpositions_back_into_history() {
    let st = alice();
    let idx = st.index("alice").unwrap();
    // Alice's only Black game went 1. e4 e5 2. Nf3 Nc6 3. Bc4 Bc5. After 1. Nf3 she has no history,
    // but 1... e5 lets 2. e4 transpose straight into it.
    let g = GameState::from_uci_moves(&["g1f3"]).unwrap();
    let cands = steering_candidates(idx, &g, &Filters::default());
    assert_eq!(cands[0].uci, "e7e5", "{cands:?}");
    assert_eq!((cands[0].games_ahead, cands[0].known_replies, cands[0].setup_games), (1, 1, 1));
    assert!(cands.iter().all(|c| c.games_ahead > 0 || c.setup_games > 0));

    let g = GameState::from_uci_moves(&["g1f3", "e7e5"]).unwrap();
    let back = session::returning_moves(idx, &g, &Filters::default());
    assert_eq!(back.len(), 1);
    assert_eq!((back[0].uci.as_str(), back[0].san.as_str(), back[0].games), ("e2e4", "e4", 1));
    let rapid = Filters { time_classes: Some(vec!["rapid".into()]), ..Default::default() };
    assert!(session::returning_moves(idx, &g, &rapid).is_empty());

    // 1. e4 e5 2. Bc4: Nc6 is both an exact transposition (3. Nf3) and a familiar setup.
    let g = GameState::from_uci_moves(&["e2e4", "e7e5", "f1c4"]).unwrap();
    let cands = steering_candidates(idx, &g, &Filters::default());
    let nc6 = cands.iter().find(|c| c.uci == "b8c6").unwrap();
    assert_eq!((nc6.games_ahead, nc6.setup_games), (1, 1));
    assert_eq!(cands[0].uci, "b8c6");
}

#[test]
fn steering_picks_the_way_back_within_tolerance() {
    let mut st = alice();
    let step = st.create_session("s", req("white"), 0).unwrap();
    assert_eq!(step.state.steer_cp, 100, "steering is on by default");
    let step = st.play("s", "g1f3", 1).unwrap();
    let request = step.engine_request.unwrap();
    assert_eq!(request.candidates[0], "e7e5");

    // Stockfish prefers d5, but e5 heads back into Alice's games and costs only 20 centipawns.
    let result = EngineResult {
        fen: request.fen.clone(),
        best: Some(EngineLine { uci: "d7d5".into(), depth: Some(14), score_cp: Some(10), score_mate: None }),
        lines: vec![EngineLine { uci: "e7e5".into(), depth: Some(14), score_cp: Some(-10), score_mate: None }],
        ..Default::default()
    };
    let s = st.engine_result("s", &result, 2).unwrap().state;
    let reply = &s.moves[1];
    assert_eq!(reply.uci, "e7e5");
    let d = reply.detail.as_ref().unwrap();
    assert_eq!((d["engine"].as_str(), d["score_cp"].as_i64()), (Some("stockfish"), Some(-10)));
    assert_eq!((d["steer"]["chosen_is_best"].as_bool(), d["steer"]["best_uci"].as_str()), (Some(false), Some("d7d5")));
    assert_eq!(s.coverage.returning_moves[0].uci, "e2e4", "and the human is told which move brings the model back");

    // Beyond the tolerance, the engine's move stands.
    st.undo("s", 3).unwrap();
    let request = st.play("s", "g1f3", 4).unwrap().engine_request.unwrap();
    let costly = EngineResult {
        lines: vec![EngineLine { uci: "e7e5".into(), depth: Some(14), score_cp: Some(-300), score_mate: None }],
        ..result.clone()
    };
    let s = st.engine_result("s", &EngineResult { fen: request.fen, ..costly }, 5).unwrap().state;
    assert_eq!(s.moves[1].uci, "d7d5");
    assert_eq!(s.moves[1].detail.as_ref().unwrap()["steer"]["rejected"], true);
}

#[test]
fn gap_filler_steers_back_even_without_an_engine() {
    let mut st = alice();
    st.create_session("s", req("white"), 0).unwrap();
    let request = st.play("s", "g1f3", 1).unwrap().engine_request.unwrap();
    let s = st.engine_result("s", &EngineResult { fen: request.fen, ..no_engine() }, 2).unwrap().state;
    let reply = &s.moves[1];
    assert_eq!((reply.source.as_str(), reply.uci.as_str()), ("engine", "e7e5"));
    let d = reply.detail.as_ref().unwrap();
    assert_eq!(d["engine"], "random");
    assert_eq!(d["steer"]["games_ahead"], 1);
    assert_eq!(d["steer"]["considered"][0]["uci"], "e7e5");

    // With steering off, the request has no candidates and the fallback carries no steering detail.
    st.create_session("off", CreateSession { steer_cp: Some(0), ..req("white") }, 0).unwrap();
    let request = st.play("off", "g1f3", 1).unwrap().engine_request.unwrap();
    assert!(request.candidates.is_empty());
    let s = st.engine_result("off", &EngineResult { fen: request.fen, ..no_engine() }, 2).unwrap().state;
    assert!(s.moves[1].detail.as_ref().unwrap().get("steer").is_none());
}

#[test]
fn games_played_here_are_remembered() {
    const NOW: i64 = 1_791_590_400_000;
    let mut st = alice();
    let all = Filters::default();
    let step = st.create_session("m", CreateSession { steer_cp: Some(0), ..req("white") }, NOW + 100).unwrap();
    assert!(step.state.remember, "remembering is on by default");
    let game_id = session::session_game_id("m");
    assert_eq!(counts(&mut st, START, &all)[1], ("d2d4".to_string(), 1));

    // Her move is remembered at once, as a game with time class yvy; the model's reply is not.
    let request = st.play("m", "d2d4", NOW + 200).unwrap().engine_request.unwrap();
    assert_eq!(counts(&mut st, START, &all), [("d2d4".to_string(), 2), ("e2e4".to_string(), 2)]);
    let d4 = st.position("alice", START, &all).unwrap().into_iter().find(|c| c.uci == "d2d4").unwrap();
    assert_eq!(d4.game_ids[0], game_id, "the game played here is the most recent");
    st.engine_result("m", &EngineResult { fen: request.fen, ..no_engine() }, NOW + 300).unwrap();
    assert!(counts(&mut st, AFTER_D4, &all).is_empty(), "the model's reply is not a memory");
    let meta = st.index("alice").unwrap().game_by_id(&game_id).unwrap().clone();
    assert_eq!((meta.time_class.as_str(), meta.result.as_str(), meta.ply_count), ("yvy", "playing", 2));
    assert_eq!(st.index("alice").unwrap().live_decisions(&game_id).unwrap().len(), 1);

    let imported_only = Filters { time_classes: Some(vec!["blitz".into(), "rapid".into()]), ..Default::default() };
    assert_eq!(counts(&mut st, START, &imported_only)[1], ("d2d4".to_string(), 1));

    // Undo takes the memory back; a different move replaces it.
    st.undo("m", NOW + 400).unwrap();
    assert_eq!(counts(&mut st, START, &all)[1], ("d2d4".to_string(), 1));
    st.play("m", "e2e4", NOW + 500).unwrap();
    assert_eq!(counts(&mut st, START, &all)[0], ("e2e4".to_string(), 3));

    st.resign("m", NOW + 600).unwrap();
    assert_eq!(st.index("alice").unwrap().game_by_id(&game_id).unwrap().result, "resigned");
    assert_eq!(st.player_summary("alice").by_time_class.get("yvy"), Some(&1));

    // A game that is not remembered leaves no trace.
    st.create_session("quiet", CreateSession { steer_cp: Some(0), remember: Some(false), ..req("white") }, NOW + 700).unwrap();
    st.play("quiet", "d2d4", NOW + 800).unwrap();
    assert!(st.index("alice").unwrap().game_by_id(&session::session_game_id("quiet")).is_none());
    assert_eq!(st.player_summary("alice").total, 5);
}

#[test]
fn switch_mode_hands_the_gap_to_the_human() {
    let mut st = alice();
    let s = st.create_session("x", CreateSession { gap_mode: Some("switch".into()), ..req("white") }, 0).unwrap().state;
    assert_eq!((s.gap_mode.as_str(), s.current_color.as_str()), ("switch", "white"));

    // Alice never faced 1. d4 as Black, so instead of an engine move the human takes over Black.
    let step = st.play("x", "d2d4", 1).unwrap();
    assert!(step.engine_request.is_none(), "no engine in switch mode");
    let s = step.state;
    assert_eq!(s.moves.len(), 1);
    assert_eq!((s.current_color.as_str(), s.turn.as_str()), ("black", "black"));
    assert_eq!(s.switches, vec![Switch { ply: 1, color: "black".into() }]);

    // The human's answer as Black is remembered as Alice's own; the model then plays White from
    // her White history: 1. d4 d5 2. c4.
    let s = st.play("x", "d7d5", 2).unwrap().state;
    assert_eq!(counts(&mut st, AFTER_D4, &Filters::default()), [("d7d5".to_string(), 1)]);
    assert_eq!((s.moves[2].source.as_str(), s.moves[2].uci.as_str()), ("history", "c2c4"));

    // 2... e6 leaves her White history too, so the human takes White back.
    let s = st.play("x", "e7e6", 3).unwrap().state;
    assert_eq!((s.current_color.as_str(), s.turn.as_str()), ("white", "white"));
    assert_eq!(s.switches[1], Switch { ply: 4, color: "white".into() });
    assert!(s.moves.iter().all(|m| m.source != "engine"));

    let s = st.undo("x", 4).unwrap().state;
    assert_eq!((s.moves.len(), s.current_color.as_str(), s.switches.len()), (3, "black", 1));
    let pgn = st.pgn("x").unwrap();
    assert!(pgn.contains("1. d4 {you take over as black} d5 2. c4 {you: 1/1}"), "pgn was:\n{pgn}");

    // Starting as Black with no usable White history switches before any move.
    let only_daily = Filters { time_classes: Some(vec!["daily".into()]), ..Default::default() };
    let s = st
        .create_session(
            "y",
            CreateSession { gap_mode: Some("switch".into()), filters: Some(only_daily), remember: Some(false), ..req("black") },
            5,
        )
        .unwrap()
        .state;
    assert_eq!((s.moves.len(), s.current_color.as_str()), (0, "white"));
    assert_eq!(s.switches, vec![Switch { ply: 0, color: "white".into() }]);
}

#[test]
fn bad_requests_are_rejected() {
    let mut st = alice();
    assert!(st.create_session("a", CreateSession { username: "bob".into(), ..req("white") }, 0).is_err());
    assert!(st.create_session("a", req("purple"), 0).is_err());
    assert!(st.create_session("a", CreateSession { gap_mode: Some("maia".into()), ..req("white") }, 0).is_err());
    st.create_session("a", req("white"), 0).unwrap();
    assert!(st.create_session("a", req("white"), 0).is_err(), "ids are unique");
    assert!(st.undo("a", 1).is_err(), "nothing to undo yet");
    assert!(st.play("nope", "e2e4", 1).is_err());
    assert!(st.position("alice", "not a fen", &Filters::default()).is_err());
    // Low targets map to Skill Level, high ones are clamped.
    assert_eq!(session::strength(698), (698, None, Some(2)));
    assert_eq!(session::strength(3500), (3190, Some(3190), None));
}

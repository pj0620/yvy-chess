// Runs the real WebAssembly backend (built by scripts/build-wasm.sh) through a game.
import { beforeAll, describe, expect, test } from "bun:test";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";

const wasmPath = fileURLToPath(new URL("../wasm/yvy_bg.wasm", import.meta.url));
const fixturePath = fileURLToPath(new URL("../../../crates/yvy-app/tests/fixtures/alice_games.json", import.meta.url));
const built = existsSync(wasmPath);
if (!built && process.env.CI) throw new Error("web/src/wasm is missing: run scripts/build-wasm.sh before the tests");

type Step = { state: any; engine_request: any | null };

describe.skipIf(!built)("wasm backend", () => {
  let mod: typeof import("../wasm/yvy.js");
  let fixture: string;

  beforeAll(async () => {
    mod = await import("../wasm/yvy.js");
    mod.initSync({ module: await Bun.file(wasmPath).arrayBuffer() });
    fixture = await Bun.file(fixturePath).text();
  });

  test("imports, plays from history, asks the engine, and reloads", () => {
    const app = new mod.App();
    const imported = app.import_month("alice", fixture);
    const stats = JSON.parse(imported.stats);
    expect([stats.seen, stats.new, stats.skipped, stats.positions]).toEqual([6, 4, 2, 10]);
    const blob = imported.blob;
    expect(blob.length).toBeGreaterThan(100);
    app.finish_loading("alice");
    expect(JSON.parse(app.player_summary("alice")).total).toBe(4);

    let step: Step = JSON.parse(
      app.create_session("t1", JSON.stringify({ username: "alice", user_color: "black", mode: "most_frequent", steer_cp: 0 })),
    );
    expect(step.state.moves[0].uci).toBe("e2e4");
    expect(step.state.moves[0].source).toBe("history");

    step = JSON.parse(app.play("t1", "g8f6"));
    expect(step.engine_request).not.toBeNull();
    expect(step.engine_request.fen).toBe(step.state.fen);
    step = JSON.parse(app.engine_result("t1", JSON.stringify({ fen: step.engine_request.fen, error: "no engine" })));
    expect(step.state.moves.length).toBe(3);
    expect(step.state.moves[2].detail.engine).toBe("random");
    expect(step.state.created_at).toBeGreaterThan(1_700_000_000_000);

    expect(() => app.play("t1", "zzzz")).toThrow(/illegal/);
    expect(app.pgn("t1")).toContain("1. e4");

    // Reload: the persisted month and session rebuild the same state.
    const saved = app.session_json("t1");
    const fresh = new mod.App();
    expect(fresh.load_month("alice", blob)).toBe(4);
    fresh.finish_loading("alice");
    fresh.load_session(saved);
    expect(JSON.parse(fresh.state("t1"))).toEqual(JSON.parse(app.state("t1")));
    expect(JSON.parse(fresh.sessions_for("alice")).length).toBe(1);
  });
});

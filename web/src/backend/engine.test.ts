import { expect, test } from "bun:test";
import { Engine, parseInfo, type WorkerLike } from "./engine";

/** A scripted stand-in for the Stockfish worker that records what it was told. */
class FakeStockfish implements WorkerLike {
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onerror: ((event: unknown) => void) | null = null;
  sent: string[] = [];
  private searchmoves: string[] = [];

  constructor(private readonly best = "e2e4") {}

  private emit(...lines: string[]) {
    queueMicrotask(() => lines.forEach((l) => this.onmessage?.({ data: l })));
  }

  postMessage(cmd: string) {
    this.sent.push(cmd);
    if (cmd === "uci") {
      this.emit("id name Fakefish 19", "option name UCI_Elo type spin default 1320 min 1320 max 3190", "uciok");
    } else if (cmd === "isready") {
      this.emit("readyok");
    } else if (cmd.startsWith("go")) {
      const m = cmd.match(/searchmoves (.*)$/);
      this.searchmoves = m ? m[1].split(" ") : [];
      if (this.searchmoves.length) {
        this.emit(
          ...this.searchmoves.map((mv, i) => `info depth 10 seldepth 12 multipv ${i + 1} score cp ${-20 * (i + 1)} nodes 1 pv ${mv} e7e5`),
          `bestmove ${this.searchmoves[0]}`,
        );
      } else {
        this.emit("info string NNUE evaluation enabled", "info depth 8 multipv 1 score cp 15 pv d2d4", `info depth 12 multipv 1 score mate 3 pv ${this.best} e7e5`, `bestmove ${this.best} ponder e7e5`);
      }
    }
  }

  terminate() {}
}

const request = {
  session_id: "s",
  fen: "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
  movetime_ms: 50,
  elo: 1500,
  uci_elo: 1500,
  skill_level: null,
  candidates: [] as string[],
};

test("parseInfo reads multipv, depth, score and the first PV move", () => {
  expect(parseInfo("info depth 14 seldepth 20 multipv 2 score cp -35 nodes 9 pv g8f6 c2c4")).toEqual({
    index: 2,
    line: { uci: "g8f6", depth: 14, score_cp: -35, score_mate: null },
  });
  expect(parseInfo("info depth 3 score mate -2 pv h7h8")?.line.score_mate).toBe(-2);
  expect(parseInfo("info string hello")).toBeNull();
  expect(parseInfo("bestmove e2e4")).toBeNull();
});

test("handshake, strength and best move", async () => {
  const fake = new FakeStockfish("e2e4");
  const engine = new Engine(() => fake, 1000);
  expect(await engine.info()).toEqual({ name: "Fakefish 19", elo_min: 1320, elo_max: 3190 });
  const result = await engine.evaluate(request);
  expect(result.error).toBeUndefined();
  expect(result.best).toEqual({ uci: "e2e4", depth: 12, score_cp: null, score_mate: 3 });
  expect(result.lines).toEqual([]);
  expect(fake.sent).toContain("setoption name UCI_Elo value 1500");
  expect(fake.sent).toContain("go movetime 50");
});

test("scores steering candidates only when the best move is not one of them", async () => {
  const fake = new FakeStockfish("d7d5");
  const engine = new Engine(() => fake, 1000);
  const result = await engine.evaluate({ ...request, uci_elo: null, skill_level: 2, candidates: ["e7e5", "c7c5"] });
  expect(fake.sent).toContain("setoption name Skill Level value 2");
  expect(fake.sent).toContain("setoption name MultiPV value 2");
  expect(fake.sent).toContain("go movetime 50 searchmoves e7e5 c7c5");
  expect(result.lines.map((l) => [l.uci, l.score_cp])).toEqual([
    ["e7e5", -20],
    ["c7c5", -40],
  ]);
  const again = await new Engine(() => new FakeStockfish("e7e5"), 1000).evaluate({ ...request, candidates: ["e7e5"] });
  expect(again.lines).toEqual([]);
});

test("a missing or silent engine becomes an error result, not an exception", async () => {
  const silent: WorkerLike = { onmessage: null, onerror: null, postMessage() {}, terminate() {} };
  const result = await new Engine(() => silent, 30).evaluate(request);
  expect(result.error).toMatch(/did not answer/);
  const broken: WorkerLike = { onmessage: null, onerror: null, postMessage() { queueMicrotask(() => broken.onerror?.({})); }, terminate() {} };
  expect((await new Engine(() => broken, 1000).evaluate(request)).error).toMatch(/could not be loaded/);
});

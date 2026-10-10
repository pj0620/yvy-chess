import { describe, expect, test } from "bun:test";
import { candidateShares, defaultElo, describeMove, monthLabel, movePairs, outcomeLabel, statusText } from "./format";
import type { MoveOut } from "../api";

describe("describeMove", () => {
  test("history moves say how often you played them", () => {
    const move: MoveOut = {
      ply: 2,
      uci: "c7c5",
      san: "c5",
      source: "history",
      detail: {
        games: 12,
        chosen: "c7c5",
        chosen_count: 8,
        mode: "sampled",
        candidates: [
          { uci: "c7c5", san: "c5", count: 8, game_ids: [] },
          { uci: "e7e5", san: "e5", count: 4, game_ids: [] },
        ],
      },
    };
    const d = describeMove(move);
    expect(d.kind).toBe("history");
    expect(d.sub).toBe("you played c5 in 8 of 12 games here (67%)");
  });

  test("engine moves show elo and depth, random fallback is labelled", () => {
    const sf = describeMove({
      ply: 4,
      uci: "d7d6",
      san: "d6",
      source: "engine",
      detail: { engine: "stockfish", elo: 1500, depth: 10, score_cp: 25, movetime_ms: 300 },
    });
    expect(sf.kind).toBe("stockfish");
    expect(sf.sub).toBe("Elo 1500 · depth 10 · eval +0.25");
    const rnd = describeMove({ ply: 4, uci: "d7d6", san: "d6", source: "engine", detail: { engine: "random" } });
    expect(rnd.kind).toBe("random");
  });

  test("steered engine moves get their own badge and explain the trade", () => {
    const d = describeMove({
      ply: 6,
      uci: "g8f6",
      san: "Nf6",
      source: "engine",
      detail: {
        engine: "stockfish",
        elo: 1500,
        depth: 11,
        score_cp: -30,
        steer: { tolerance_cp: 100, chosen_is_best: false, games_ahead: 12, known_replies: 3, best_uci: "d7d5", best_score_cp: -5 },
      },
    });
    expect(d.kind).toBe("steer");
    expect(d.sub).toBe("Nf6 heads toward 12 of your games one move ahead · Elo 1500 · depth 11 · eval -0.30 instead of -0.05 for d7d5");
    const best = describeMove({
      ply: 6, uci: "d7d5", san: "d5", source: "engine",
      detail: { engine: "stockfish", elo: 1500, depth: 11, score_cp: -5, steer: { tolerance_cp: 100, chosen_is_best: true, games_ahead: 4 } },
    });
    expect(best.kind).toBe("stockfish");
    expect(best.sub).toContain("also heads toward 4 of your games");
    const setup = describeMove({
      ply: 8, uci: "f8g7", san: "Bg7", source: "engine",
      detail: { engine: "stockfish", elo: 1500, depth: 10, score_cp: 12, steer: { tolerance_cp: 100, chosen_is_best: false, games_ahead: 0, setup_games: 23, best_uci: "d7d5", best_score_cp: 30 } },
    });
    expect(setup.kind).toBe("steer");
    expect(setup.sub).toBe("Bg7 recreates a piece setup you had in 23 of your games · Elo 1500 · depth 10 · eval +0.12 instead of +0.30 for d7d5");
  });
});

test("candidateShares sums to 100", () => {
  const shares = candidateShares([
    { uci: "a", san: "a", count: 3, game_ids: [] },
    { uci: "b", san: "b", count: 1, game_ids: [] },
  ]);
  expect(shares).toEqual([75, 25]);
  expect(candidateShares([])).toEqual([]);
});

test("defaultElo follows the selected time classes then the fallback order", () => {
  const ratings = { blitz: 1400, rapid: 1600, bullet: 1200 };
  expect(defaultElo(ratings, ["blitz"])).toBe(1400);
  expect(defaultElo(ratings, ["daily", "bullet"])).toBe(1200);
  expect(defaultElo(ratings, [])).toBe(1600);
  expect(defaultElo(undefined, [])).toBe(1500);
});

test("movePairs groups plies into numbered pairs", () => {
  const mk = (ply: number): MoveOut => ({ ply, uci: "x", san: `m${ply}`, source: "player" });
  const pairs = movePairs([mk(1), mk(2), mk(3)]);
  expect(pairs.length).toBe(2);
  expect(pairs[1].white?.san).toBe("m3");
  expect(pairs[1].black).toBeUndefined();
});

test("statusText", () => {
  expect(statusText("checkmate", "win", "white")).toBe("Checkmate. You win.");
  expect(statusText("playing", null, "white")).toBe("");
  expect(statusText("resigned", "loss", "black")).toBe("You resigned as black.");
});

test("monthLabel and outcomeLabel", () => {
  expect(monthLabel("2025-06-23T10:00:00Z", "en-US")).toBe("Jun 2025");
  expect(monthLabel(null)).toBe("");
  expect(monthLabel("garbage")).toBe("");
  expect(monthLabel(Date.UTC(2025, 5, 23), "en-US")).toBe("Jun 2025");
  expect(outcomeLabel("playing", null)).toBe("in progress");
  expect(outcomeLabel("checkmate", "win")).toBe("won");
  expect(outcomeLabel("resigned", "loss")).toBe("resigned");
  expect(outcomeLabel("stalemate", "draw")).toBe("draw");
});

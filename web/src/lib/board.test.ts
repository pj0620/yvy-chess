import { expect, test } from "bun:test";
import { applyUci, fenToPosition, positionAtPly } from "./board";

const START = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

test("fenToPosition reads the start position", () => {
  const p = fenToPosition(START);
  expect(Object.keys(p).length).toBe(32);
  expect(p.e2?.pieceType).toBe("wP");
  expect(p.e8?.pieceType).toBe("bK");
  expect(p.e4).toBeUndefined();
});

test("applyUci moves pieces, castles, captures en passant and promotes", () => {
  const start = fenToPosition(START);
  const afterE4 = applyUci(start, "e2e4");
  expect(afterE4.e2).toBeUndefined();
  expect(afterE4.e4?.pieceType).toBe("wP");
  expect(start.e2?.pieceType).toBe("wP");

  const castled = applyUci(fenToPosition("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1"), "e1g1");
  expect(castled.g1?.pieceType).toBe("wK");
  expect(castled.f1?.pieceType).toBe("wR");
  expect(castled.h1).toBeUndefined();

  const ep = applyUci(fenToPosition("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 2"), "e5d6");
  expect(ep.d6?.pieceType).toBe("wP");
  expect(ep.d5).toBeUndefined();

  const promoted = applyUci(fenToPosition("4k3/P7/8/8/8/8/8/4K3 w - - 0 1"), "a7a8q");
  expect(promoted.a8?.pieceType).toBe("wQ");
});

test("positionAtPly replays the first N moves", () => {
  const moves = [{ uci: "e2e4" }, { uci: "e7e5" }, { uci: "g1f3" }];
  expect(positionAtPly(moves, 0)).toEqual(fenToPosition(START));
  const after2 = positionAtPly(moves, 2);
  expect(after2.e4?.pieceType).toBe("wP");
  expect(after2.e5?.pieceType).toBe("bP");
  expect(after2.g1?.pieceType).toBe("wN");
  const after3 = positionAtPly(moves, 3);
  expect(after3.f3?.pieceType).toBe("wN");
  expect(positionAtPly(moves, 99)).toEqual(after3);
});

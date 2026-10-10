import type { PositionDataType } from "react-chessboard";

const FILES = "abcdefgh";

/** Piece placement from a FEN as the square map react-chessboard accepts. */
export function fenToPosition(fen: string): PositionDataType {
  const placement = fen.split(" ")[0] ?? "";
  const position: PositionDataType = {};
  placement.split("/").forEach((rank, i) => {
    let file = 0;
    for (const ch of rank) {
      if (ch >= "1" && ch <= "8") {
        file += Number(ch);
      } else {
        const square = `${FILES[file]}${8 - i}`;
        const color = ch === ch.toUpperCase() ? "w" : "b";
        position[square] = { pieceType: `${color}${ch.toUpperCase()}` };
        file += 1;
      }
    }
  });
  return position;
}

/**
 * Applies a UCI move to a position map, including castling rook moves, en passant captures
 * and promotions, so the board can show the player's move before the server confirms it.
 */
export function applyUci(position: PositionDataType, uci: string): PositionDataType {
  const from = uci.slice(0, 2);
  const to = uci.slice(2, 4);
  const promotion = uci[4];
  const next: PositionDataType = { ...position };
  const piece = next[from];
  if (!piece) return next;
  delete next[from];
  const kind = piece.pieceType[1];
  if (kind === "P" && from[0] !== to[0] && !position[to]) {
    delete next[`${to[0]}${from[1]}`];
  }
  if (kind === "K" && Math.abs(from.charCodeAt(0) - to.charCodeAt(0)) === 2) {
    const rank = from[1];
    const kingside = to[0] === "g";
    const rookFrom = `${kingside ? "h" : "a"}${rank}`;
    const rookTo = `${kingside ? "f" : "d"}${rank}`;
    const rook = next[rookFrom];
    delete next[rookFrom];
    if (rook) next[rookTo] = rook;
  }
  next[to] = promotion ? { pieceType: `${piece.pieceType[0]}${promotion.toUpperCase()}` } : piece;
  return next;
}

export const START_FEN = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

/** The position after the first `ply` moves of a game, replayed from the start position. */
export function positionAtPly(moves: { uci: string }[], ply: number): PositionDataType {
  let position = fenToPosition(START_FEN);
  for (const m of moves.slice(0, Math.max(0, ply))) {
    position = applyUci(position, m.uci);
  }
  return position;
}

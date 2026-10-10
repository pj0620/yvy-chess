import type { Candidate, EngineDetail, HistoryDetail, MoveOut } from "../api";

export function isHistory(detail: MoveOut["detail"]): detail is HistoryDetail {
  return !!detail && "candidates" in detail;
}

export function isEngine(detail: MoveOut["detail"]): detail is EngineDetail {
  return !!detail && "engine" in detail;
}

/** Short label for a move's provenance badge. */
export type MoveKind = "you" | "history" | "stockfish" | "steer" | "random";

export const STEER_LEVELS: { value: number; label: string; hint: string }[] = [
  { value: 0, label: "Off", hint: "Stockfish plays its best move" },
  { value: 50, label: "Light", hint: "give up at most half a pawn to head back" },
  { value: 100, label: "Normal", hint: "give up at most a pawn to head back" },
  { value: 250, label: "Strong", hint: "give up up to two and a half pawns to head back" },
];

export const GAP_MODES: { value: "stockfish" | "switch"; label: string; hint: string }[] = [
  { value: "stockfish", label: "Stockfish fills in", hint: "the engine plays the gap at your rating and steers back toward your games" },
  { value: "switch", label: "Switch sides", hint: "you take over the model's side and play the position yourself; the model plays your old side from your history" },
];

export function steerLabel(cp: number): string {
  return STEER_LEVELS.find((l) => l.value === cp)?.label ?? `${cp} cp`;
}

export function describeMove(move: MoveOut): { kind: MoveKind; label: string; sub?: string } {
  if (move.source === "player") return { kind: "you", label: "You" };
  if (move.source === "history" && isHistory(move.detail)) {
    const d = move.detail;
    const share = d.games > 0 ? Math.round((100 * d.chosen_count) / d.games) : 0;
    return {
      kind: "history",
      label: "From your games",
      sub: `you played ${move.san} in ${d.chosen_count} of ${d.games} ${d.games === 1 ? "game" : "games"} here (${share}%)`,
    };
  }
  if (move.source === "engine" && isEngine(move.detail)) {
    const d = move.detail;
    const steer = d.steer;
    const steered = !!steer && steer.chosen_is_best === false && ((steer.games_ahead ?? 0) > 0 || (steer.setup_games ?? 0) > 0);
    const why = steered ? steerReason(move.san, steer.games_ahead ?? 0, steer.setup_games ?? 0) : "";
    if (d.engine === "random") {
      if (steered) {
        return { kind: "steer", label: "Steering back", sub: `no engine available; ${why}` };
      }
      return { kind: "random", label: "Random move", sub: "no engine available, so a random legal move was played" };
    }
    const parts: string[] = [];
    if (d.skill_level != null) parts.push(`skill ${d.skill_level}`);
    else if (d.elo) parts.push(`Elo ${d.elo}`);
    if (d.depth) parts.push(`depth ${d.depth}`);
    if (d.score_mate != null) parts.push(`mate in ${Math.abs(d.score_mate)}`);
    else if (d.score_cp != null) parts.push(`eval ${formatCp(d.score_cp)}`);
    if (steered) {
      const vsBest =
        d.score_cp != null && steer.best_score_cp != null ? ` instead of ${formatCp(steer.best_score_cp)} for ${steer.best_uci}` : "";
      return { kind: "steer", label: "Stockfish, steering back", sub: `${why} · ${parts.join(" · ")}${vsBest}` };
    }
    if (steer?.chosen_is_best) {
      parts.push((steer.games_ahead ?? 0) > 0 ? `also heads toward ${steer.games_ahead} of your games` : `also a setup from ${steer.setup_games} of your games`);
    }
    else if (steer?.rejected) parts.push("no way back to your games within tolerance");
    return { kind: "stockfish", label: "Stockfish", sub: parts.join(" · ") };
  }
  return { kind: move.source === "history" ? "history" : "stockfish", label: move.source };
}

function steerReason(san: string, gamesAhead: number, setupGames: number): string {
  if (gamesAhead > 0) return `${san} heads toward ${gamesAhead} of your ${gamesAhead === 1 ? "game" : "games"} one move ahead`;
  return `${san} recreates a piece setup you had in ${setupGames} of your ${setupGames === 1 ? "game" : "games"}`;
}

export function formatCp(cp: number): string {
  const pawns = cp / 100;
  return `${pawns > 0 ? "+" : ""}${pawns.toFixed(2)}`;
}

/** Shares of each candidate, in the order given, as percentages that sum to about 100. */
export function candidateShares(candidates: Candidate[]): number[] {
  const total = candidates.reduce((n, c) => n + c.count, 0);
  if (total === 0) return candidates.map(() => 0);
  return candidates.map((c) => (100 * c.count) / total);
}

/**
 * Mirrors the server default: rating of the first selected time class, otherwise rapid,
 * blitz, bullet, daily, otherwise 1500.
 */
export function defaultElo(ratings: Record<string, number> | undefined, timeClasses: string[]): number {
  const order = timeClasses.length > 0 ? timeClasses : ["rapid", "blitz", "bullet", "daily"];
  for (const c of order) {
    const r = ratings?.[c];
    if (typeof r === "number") return r;
  }
  return 1500;
}

/** "1. e4 e5 2. Nf3" style pairs for the move list. */
export function movePairs(moves: MoveOut[]): { number: number; white?: MoveOut; black?: MoveOut }[] {
  const pairs: { number: number; white?: MoveOut; black?: MoveOut }[] = [];
  for (const m of moves) {
    const idx = Math.floor((m.ply - 1) / 2);
    if (!pairs[idx]) pairs[idx] = { number: idx + 1 };
    if (m.ply % 2 === 1) pairs[idx].white = m;
    else pairs[idx].black = m;
  }
  return pairs;
}

export function statusText(status: string, result: string | null, userColor: string): string {
  switch (status) {
    case "playing":
      return "";
    case "checkmate":
      return result === "win" ? "Checkmate. You win." : "Checkmate. You lost.";
    case "stalemate":
      return "Stalemate. Draw.";
    case "insufficient_material":
      return "Draw by insufficient material.";
    case "threefold_repetition":
      return "Draw by threefold repetition.";
    case "fifty_move_rule":
      return "Draw by the fifty-move rule.";
    case "resigned":
      return `You resigned as ${userColor}.`;
    default:
      return status;
  }
}

/** "Jun 2025" for a timestamp, or an empty string when unknown. */
export function monthLabel(when: number | string | null | undefined, locale?: string): string {
  if (when == null || when === "") return "";
  const d = new Date(when);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleDateString(locale, { month: "short", year: "numeric" });
}

/** "2026-10-10" for a timestamp in ms, or an empty string. */
export function dayLabel(ms: number | null | undefined): string {
  if (ms == null) return "";
  const d = new Date(ms);
  return Number.isNaN(d.getTime()) ? "" : d.toISOString().slice(0, 10);
}

/** Short outcome for a game list: "won", "lost", "draw" or "in progress". */
export function outcomeLabel(status: string, result: string | null): string {
  if (status === "playing") return "in progress";
  if (result === "win") return "won";
  if (result === "loss") return status === "resigned" ? "resigned" : "lost";
  return "draw";
}

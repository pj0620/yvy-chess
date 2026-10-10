import * as backend from "./backend";

// Types and calls for the backend. The backend is Rust compiled to WebAssembly and runs in the
// page (src/backend); the shapes mirror crates/yvy-app/src/session.rs.

export type Color = "white" | "black";
export type Source = "player" | "history" | "engine";
export type Mode = "sampled" | "most_frequent";
/** What happens when the model has no history for the position it must move in. */
export type GapMode = "stockfish" | "switch";

/** The human took over `color` after `ply` moves. */
export interface Switch {
  ply: number;
  color: Color;
}

export interface Candidate {
  uci: string;
  san: string;
  count: number;
  game_ids: string[];
  /** ms since the epoch (older games played here may carry an ISO string). */
  last_played?: number | string | null;
}

export interface HistoryDetail {
  games: number;
  chosen: string;
  chosen_count: number;
  mode: Mode;
  candidates: Candidate[];
}

export interface SteerConsidered {
  uci: string;
  san: string;
  games_ahead: number;
  known_replies: number;
  setup_games?: number;
  score_cp?: number | null;
  score_mate?: number | null;
}

/** Why a gap-filling move was (or was not) steered back toward your games. */
export interface SteerDetail {
  tolerance_cp: number;
  candidates?: number;
  chosen_is_best?: boolean;
  rejected?: boolean;
  games_ahead?: number;
  known_replies?: number;
  setup_games?: number;
  best_uci?: string;
  best_score_cp?: number | null;
  best_score_mate?: number | null;
  considered?: SteerConsidered[];
}

export interface EngineDetail {
  engine: "stockfish" | "random";
  elo?: number;
  skill_level?: number | null;
  depth?: number | null;
  score_cp?: number | null;
  score_mate?: number | null;
  movetime_ms?: number;
  reason?: string;
  steer?: SteerDetail;
}

export interface MoveOut {
  ply: number;
  uci: string;
  san: string;
  source: Source;
  detail?: HistoryDetail | EngineDetail;
}

export interface ReturningMove {
  uci: string;
  san: string;
  games: number;
}

export interface Coverage {
  in_history: boolean;
  games: number;
  candidates: Candidate[];
  /** Moves for the side to move after which the position is back in your games. */
  returning_moves: ReturningMove[];
}

export interface Filters {
  time_classes?: string[];
  /** ms since the epoch */
  date_from?: number;
  /** ms since the epoch */
  date_to?: number;
  rated?: boolean;
}

export interface SessionState {
  id: string;
  username: string;
  user_color: Color;
  engine_elo: number;
  mode: Mode;
  filters: Filters;
  steer_cp: number;
  remember: boolean;
  gap_mode: GapMode;
  /** The side you play right now; differs from user_color after a switch. */
  current_color: Color;
  switches: Switch[];
  status: string;
  result: "win" | "loss" | "draw" | null;
  fen: string;
  turn: Color;
  ply: number;
  legal_moves: string[];
  moves: MoveOut[];
  coverage: Coverage;
  created_at: number;
  updated_at: number;
}

export interface SessionSummary {
  id: string;
  user_color: Color;
  engine_elo: number;
  mode: Mode;
  steer_cp: number;
  remember: boolean;
  gap_mode: GapMode;
  current_color: Color;
  status: string;
  result: "win" | "loss" | "draw" | null;
  plies: number;
  created_at: number;
  updated_at: number;
}

export interface CreateSessionBody {
  username: string;
  user_color: Color;
  engine_elo?: number;
  mode?: Mode;
  filters?: Filters;
  steer_cp?: number;
  remember?: boolean;
  gap_mode?: GapMode;
}

export type { EngineStatus, GameCounts, PlayerSummary, Progress } from "./backend";

/** Errors carry an HTTP-like status so pages can tell "not imported yet" (404) apart. */
export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

async function call<T>(fn: () => Promise<T>): Promise<T> {
  try {
    return await fn();
  } catch (e) {
    if (e instanceof backend.NotFoundError) throw new ApiError(404, e.message);
    if (e instanceof ApiError) throw e;
    throw new ApiError(400, e instanceof Error ? e.message : String(e));
  }
}

export const api = {
  player: (username: string) => call(() => backend.player(username)),
  startImport: (username: string, months?: number) =>
    call(async () => ({ status: "started", progress: await backend.startImport(username, { months }) })),
  engine: () => backend.engineStatus(),
  sessions: (username: string) => call(() => backend.sessions<SessionSummary[]>(username)),
  createSession: (body: CreateSessionBody) => call(() => backend.createSession<SessionState>(body as CreateSessionBody & Record<string, unknown>)),
  session: (id: string) => call(() => backend.session<SessionState>(id)),
  move: (id: string, uci: string) => call(() => backend.move<SessionState>(id, uci)),
  undo: (id: string) => call(() => backend.undo<SessionState>(id)),
  resign: (id: string) => call(() => backend.resign<SessionState>(id)),
  pgn: (id: string) => call(() => backend.pgn(id)),
  exportBackup: (username: string) => call(() => backend.exportBackup(username)),
  importBackup: (text: string) => call(() => backend.importBackup(text)),
};

export const subscribeImport = backend.subscribeImport;

/** Games played in this app are stored under this time class. */
export const APP_TIME_CLASS = "yvy";
export const TIME_CLASS_LABELS: Record<string, string> = { [APP_TIME_CLASS]: "You v You" };
export function timeClassLabel(c: string): string {
  return TIME_CLASS_LABELS[c] ?? c;
}
/** Candidate game ids are chess.com URLs or games played here; map the latter to the game page. */
export function gameLink(id: string): { href: string; external: boolean } {
  const prefix = "yvy:session:";
  return id.startsWith(prefix) ? { href: `#/g/${id.slice(prefix.length)}`, external: false } : { href: id, external: true };
}

/** Saves text as a file in the user's downloads. */
export function download(filename: string, text: string, type = "text/plain") {
  const url = URL.createObjectURL(new Blob([text], { type }));
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

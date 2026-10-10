// Typed client for the Rust API. Shapes mirror crates/yvy-server/src/{session,import,api}.rs.

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
  last_played?: string | null;
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
  date_from?: string;
  date_to?: string;
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
  created_at: string;
}

export interface Progress {
  username: string;
  status: "running" | "done" | "failed";
  phase: string;
  archives_total: number;
  archives_done: number;
  games_seen: number;
  games_new: number;
  games_skipped: number;
  positions_new: number;
  errors: string[];
  message: string | null;
  started_at: string;
  finished_at: string | null;
}

export interface GameCounts {
  total: number;
  white: number;
  black: number;
  positions: number;
  by_time_class: Record<string, number>;
  first_game: string | null;
  last_game: string | null;
}

export interface PlayerSummary {
  username: string;
  imported: boolean;
  player_id?: number | null;
  joined?: string | null;
  last_import_at?: string | null;
  ratings?: Record<string, number>;
  games?: GameCounts;
  import?: Progress | null;
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
  created_at: string;
  updated_at: string;
}

export interface EngineInfo {
  available: boolean;
  name?: string;
  path?: string;
  elo_min?: number;
  elo_max?: number;
  movetime_ms?: number;
  error?: string;
  hint?: string;
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

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const res = await fetch(path, {
    ...init,
    headers: { "content-type": "application/json", ...(init.headers ?? {}) },
  });
  if (!res.ok) {
    let message = `${res.status} ${res.statusText}`;
    try {
      const body = (await res.json()) as { error?: string };
      if (body.error) message = body.error;
    } catch {
      // not JSON; keep the status text
    }
    throw new ApiError(res.status, message);
  }
  return (await res.json()) as T;
}

const u = encodeURIComponent;

export const api = {
  player: (username: string) => request<PlayerSummary>(`/api/players/${u(username)}`),
  startImport: (username: string, months?: number) =>
    request<{ status: string; progress: Progress }>(
      `/api/players/${u(username)}/import${months ? `?months=${months}` : ""}`,
      { method: "POST" },
    ),
  importStatus: (username: string) => request<Progress>(`/api/players/${u(username)}/import`),
  engine: () => request<EngineInfo>("/api/engine"),
  sessions: (username: string, limit = 30) => request<SessionSummary[]>(`/api/players/${u(username)}/sessions?limit=${limit}`),
  createSession: (body: CreateSessionBody) =>
    request<SessionState>("/api/sessions", { method: "POST", body: JSON.stringify(body) }),
  session: (id: string) => request<SessionState>(`/api/sessions/${u(id)}`),
  move: (id: string, uci: string) =>
    request<SessionState>(`/api/sessions/${u(id)}/moves`, { method: "POST", body: JSON.stringify({ uci }) }),
  undo: (id: string) => request<SessionState>(`/api/sessions/${u(id)}/undo`, { method: "POST" }),
  resign: (id: string) => request<SessionState>(`/api/sessions/${u(id)}/resign`, { method: "POST" }),
  pgnUrl: (id: string) => `/api/sessions/${u(id)}/pgn`,
};

/** Subscribes to import progress. Returns a function that closes the stream. */
export function subscribeImport(username: string, onProgress: (p: Progress) => void, onError?: () => void) {
  const es = new EventSource(`/api/players/${u(username)}/import/events`);
  es.onmessage = (e) => {
    const p = JSON.parse(e.data) as Progress;
    onProgress(p);
    if (p.status !== "running") es.close();
  };
  es.onerror = () => {
    es.close();
    onError?.();
  };
  return () => es.close();
}

/** Games played in this app are stored under this time class. */
export const APP_TIME_CLASS = "yvy";
export const TIME_CLASS_LABELS: Record<string, string> = { [APP_TIME_CLASS]: "You v You" };
export function timeClassLabel(c: string): string {
  return TIME_CLASS_LABELS[c] ?? c;
}
/** Candidate game ids are chess.com URLs or app sessions; map the latter to the game page. */
export function gameLink(id: string): { href: string; external: boolean } {
  const prefix = "yvy:session:";
  return id.startsWith(prefix) ? { href: `/g/${id.slice(prefix.length)}`, external: false } : { href: id, external: true };
}

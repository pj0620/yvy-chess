// The app's backend, running entirely in the browser: the Rust logic compiled to WebAssembly,
// the browser database for persistence, chess.com for imports and Stockfish in a worker.
// The functions here are what the UI calls; they replace the old HTTP API one for one.

import init, { App } from "../wasm/yvy.js";
import * as chesscom from "./chesscom";
import * as db from "./db";
import { Engine, type EngineInfo, type EngineRequest } from "./engine";

export class NotFoundError extends Error {
  readonly status = 404;
}

// ---- WebAssembly ---------------------------------------------------------------------------

let appPromise: Promise<App> | null = null;

function app(): Promise<App> {
  appPromise ??= (async () => {
    await init({ module_or_path: new URL("yvy_bg.wasm", document.baseURI) });
    return new App();
  })();
  return appPromise;
}

const normalize = (u: string) => u.trim().toLowerCase();

// ---- Loading a player's history into memory ------------------------------------------------

const loaded = new Map<string, Promise<void>>();

/** Loads a player's stored months and games played here into the wasm index, once. */
function ensurePlayer(username: string): Promise<void> {
  const u = normalize(username);
  let p = loaded.get(u);
  if (!p) {
    p = (async () => {
      const a = await app();
      let stale = false;
      for (const m of await db.monthsFor(u)) {
        try {
          a.load_month(u, m.blob);
        } catch {
          // Stored by an older version of the app: drop it so the next import fetches it again.
          stale = true;
          await db.deleteMonth(m.key);
        }
      }
      a.finish_loading(u);
      for (const s of await db.sessionsFor(u)) {
        if (!a.has_session(s.id)) a.load_session(JSON.stringify(s));
      }
      if (stale) void startImport(u);
    })();
    loaded.set(u, p);
    p.catch(() => loaded.delete(u));
  }
  return p;
}

// ---- Import --------------------------------------------------------------------------------

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
  started_at: number;
  finished_at: number | null;
}

const imports = new Map<string, Progress>();
const importListeners = new Map<string, Set<(p: Progress) => void>>();

function publish(p: Progress) {
  const snapshot = { ...p, errors: [...p.errors] };
  imports.set(p.username, snapshot);
  importListeners.get(p.username)?.forEach((l) => l(snapshot));
}

export function importProgress(username: string): Progress | null {
  return imports.get(normalize(username)) ?? null;
}

/** Calls `listener` with every progress update of the player's import. Returns an unsubscribe. */
export function subscribeImport(username: string, listener: (p: Progress) => void): () => void {
  const u = normalize(username);
  let set = importListeners.get(u);
  if (!set) importListeners.set(u, (set = new Set()));
  set.add(listener);
  const current = imports.get(u);
  if (current) listener(current);
  return () => set?.delete(listener);
}

/**
 * Downloads the player's games from chess.com, month by month, into the index and the browser
 * database. Past months already stored are skipped, so a second run only fetches what is new.
 * Throws NotFoundError for unknown usernames; otherwise runs in the background.
 */
export async function startImport(username: string, opts: { months?: number } = {}): Promise<Progress> {
  const u = normalize(username);
  const running = imports.get(u);
  if (running?.status === "running") return running;
  if (!/^[a-z0-9_-]+$/.test(u)) throw new Error("That does not look like a chess.com username.");
  const profile = await chesscom.player(u);
  if (!profile) throw new NotFoundError(`chess.com user "${u}" was not found`);

  const p: Progress = {
    username: u,
    status: "running",
    phase: "starting",
    archives_total: 0,
    archives_done: 0,
    games_seen: 0,
    games_new: 0,
    games_skipped: 0,
    positions_new: 0,
    errors: [],
    message: null,
    started_at: Date.now(),
    finished_at: null,
  };
  publish(p);

  void (async () => {
    try {
      await db.requestPersistence();
      const existing = await db.getPlayer(u);
      const ratings = await chesscom.ratings(u).catch(() => existing?.ratings ?? {});
      await db.putPlayer({
        username: u,
        player_id: profile.player_id,
        joined: profile.joined,
        ratings,
        last_import_at: existing?.last_import_at ?? null,
      });
      await ensurePlayer(u);
      const a = await app();

      p.phase = "archives";
      publish(p);
      let urls = (await chesscom.archives(u)).filter((url) => chesscom.archiveMonth(url));
      if (opts.months) urls = urls.slice(-opts.months);
      const stored = new Map((await db.monthsFor(u)).map((m) => [m.month, m]));
      const thisMonth = chesscom.currentMonth();
      p.archives_total = urls.length;
      p.phase = "games";
      publish(p);

      for (const url of urls) {
        const month = chesscom.archiveMonth(url)!;
        if (!stored.get(month)?.complete) {
          try {
            const text = await chesscom.monthText(url, month >= thisMonth);
            const imported = a.import_month(u, text);
            const stats = JSON.parse(imported.stats) as {
              seen: number;
              new: number;
              skipped: number;
              positions: number;
              errors: string[];
            };
            await db.putMonth({
              key: `${u}|${month}`,
              username: u,
              month,
              blob: imported.blob,
              complete: month < thisMonth,
              games: stats.seen,
              saved_at: Date.now(),
            });
            p.games_seen += stats.seen;
            p.games_new += stats.new;
            p.games_skipped += stats.skipped;
            p.positions_new += stats.positions;
            for (const e of stats.errors) if (p.errors.length < 50) p.errors.push(e);
          } catch (e) {
            if (p.errors.length < 50) p.errors.push(`${month}: ${(e as Error).message}`);
          }
        }
        p.archives_done += 1;
        publish(p);
      }
      a.finish_loading(u);
      const record = await db.getPlayer(u);
      if (record) await db.putPlayer({ ...record, last_import_at: Date.now() });
      p.status = "done";
      p.phase = "done";
    } catch (e) {
      p.status = "failed";
      p.message = (e as Error).message;
    }
    p.finished_at = Date.now();
    publish(p);
  })();
  return { ...p };
}

// ---- Players -------------------------------------------------------------------------------

export interface GameCounts {
  total: number;
  white: number;
  black: number;
  positions: number;
  by_time_class: Record<string, number>;
  first_game: number | null;
  last_game: number | null;
}

export interface PlayerSummary {
  username: string;
  imported: boolean;
  player_id?: number | null;
  joined?: number | null;
  last_import_at?: number | null;
  ratings?: Record<string, number>;
  games?: GameCounts;
  import?: Progress | null;
}

export async function player(username: string): Promise<PlayerSummary> {
  const u = normalize(username);
  const record = await db.getPlayer(u);
  const progress = importProgress(u);
  if (!record) {
    if (progress) return { username: u, imported: false, import: progress };
    throw new NotFoundError(`${u} has not been imported`);
  }
  await ensurePlayer(u);
  const games = JSON.parse((await app()).player_summary(u)) as GameCounts;
  return { ...record, imported: true, games, import: progress };
}

// ---- Engine --------------------------------------------------------------------------------

const engine = new Engine();

export interface EngineStatus extends Partial<EngineInfo> {
  available: boolean;
  error?: string;
}

export async function engineStatus(): Promise<EngineStatus> {
  try {
    return { available: true, ...(await engine.info()) };
  } catch (e) {
    return { available: false, error: (e as Error).message };
  }
}

// ---- Games ---------------------------------------------------------------------------------

// The shapes come from the Rust backend (crates/yvy-app/src/session.rs); see ../api.ts.
type Step = { state: { id: string; username: string } & Record<string, unknown>; engine_request: EngineRequest | null };

const sessionLocks = new Map<string, Promise<unknown>>();

/** Serializes operations on one game so engine round trips never interleave. */
function locked<T>(id: string, fn: () => Promise<T>): Promise<T> {
  const prev = sessionLocks.get(id) ?? Promise.resolve();
  const next = prev.then(fn, fn);
  sessionLocks.set(id, next.catch(() => undefined));
  return next;
}

async function persist(a: App, id: string) {
  await db.putSession(JSON.parse(a.session_json(id)));
}

/** Persists a step and, while the model needs the engine, runs it and applies the answer. */
async function settle<T>(a: App, id: string, stepJson: string): Promise<T> {
  let step = JSON.parse(stepJson) as Step;
  await persist(a, id);
  while (step.engine_request) {
    const result = await engine.evaluate(step.engine_request);
    step = JSON.parse(a.engine_result(id, JSON.stringify(result))) as Step;
    await persist(a, id);
  }
  return step.state as T;
}

async function loadSession(id: string): Promise<App> {
  const a = await app();
  if (!a.has_session(id)) {
    const record = await db.getSession(id);
    if (!record) throw new NotFoundError("game not found");
    await ensurePlayer(record.username);
    if (!a.has_session(id)) a.load_session(JSON.stringify(record));
  }
  return a;
}

export function session<T>(id: string): Promise<T> {
  // A reload can interrupt a pending model move; resume finishes it.
  return locked(id, async () => {
    const a = await loadSession(id);
    return settle<T>(a, id, a.resume(id));
  });
}

export async function createSession<T>(request: { username: string } & Record<string, unknown>): Promise<T> {
  const u = normalize(request.username);
  await ensurePlayer(u);
  const a = await app();
  const record = await db.getPlayer(u);
  const id = crypto.randomUUID();
  return locked(id, () => settle<T>(a, id, a.create_session(id, JSON.stringify({ ratings: record?.ratings, ...request, username: u }))));
}

export function move<T>(id: string, uci: string): Promise<T> {
  return locked(id, async () => {
    const a = await loadSession(id);
    return settle<T>(a, id, a.play(id, uci));
  });
}

export function undo<T>(id: string): Promise<T> {
  return locked(id, async () => {
    const a = await loadSession(id);
    return settle<T>(a, id, a.undo(id));
  });
}

export function resign<T>(id: string): Promise<T> {
  return locked(id, async () => {
    const a = await loadSession(id);
    return settle<T>(a, id, a.resign(id));
  });
}

export async function sessions<T>(username: string): Promise<T> {
  const u = normalize(username);
  await ensurePlayer(u);
  return JSON.parse((await app()).sessions_for(u)) as T;
}

export async function pgn(id: string): Promise<string> {
  return (await loadSession(id)).pgn(id);
}

// ---- Backup --------------------------------------------------------------------------------

export interface Backup {
  format: "yvy-chess-backup";
  version: 1;
  username: string;
  exported_at: number;
  sessions: db.SessionRecord[];
}

/** The games played here, which exist nowhere else. Imported games can be fetched again. */
export async function exportBackup(username: string): Promise<Backup> {
  const u = normalize(username);
  return { format: "yvy-chess-backup", version: 1, username: u, exported_at: Date.now(), sessions: await db.sessionsFor(u) };
}

/** Restores games from a backup file. Returns how many were added; existing games are kept. */
export async function importBackup(text: string): Promise<{ username: string; added: number }> {
  let backup: Backup;
  try {
    backup = JSON.parse(text) as Backup;
  } catch {
    throw new Error("That file is not a You v You backup.");
  }
  if (backup?.format !== "yvy-chess-backup" || !Array.isArray(backup.sessions)) throw new Error("That file is not a You v You backup.");
  const a = await app();
  let added = 0;
  for (const s of backup.sessions) {
    await ensurePlayer(s.username);
    if (a.has_session(s.id) || (await db.getSession(s.id))) continue;
    a.load_session(JSON.stringify(s));
    await persist(a, s.id);
    added += 1;
  }
  return { username: normalize(backup.username ?? ""), added };
}

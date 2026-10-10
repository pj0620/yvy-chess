// The browser's IndexedDB holds everything the app knows: one blob per imported chess.com month
// (produced by the Rust backend), each player's profile, and every game played here.

const DB_NAME = "yvy-chess";
const DB_VERSION = 1;

export interface PlayerRecord {
  username: string;
  player_id: number | null;
  /** Account creation, ms since the epoch. */
  joined: number | null;
  ratings: Record<string, number>;
  last_import_at: number | null;
}

export interface MonthRecord {
  /** `${username}|${YYYY-MM}` */
  key: string;
  username: string;
  month: string;
  blob: Uint8Array;
  /** Past months never change, so a complete month is not fetched again. */
  complete: boolean;
  games: number;
  saved_at: number;
}

/** A session exactly as serialized by the Rust backend (it has `id` and `username`). */
export type SessionRecord = { id: string; username: string } & Record<string, unknown>;

let dbPromise: Promise<IDBDatabase> | null = null;

function open(): Promise<IDBDatabase> {
  if (dbPromise) return dbPromise;
  dbPromise = new Promise((resolve, reject) => {
    if (typeof indexedDB === "undefined") {
      reject(new Error("This browser has no IndexedDB, so You v You cannot store your games."));
      return;
    }
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = () => {
      const db = req.result;
      db.createObjectStore("players", { keyPath: "username" });
      db.createObjectStore("months", { keyPath: "key" }).createIndex("username", "username");
      db.createObjectStore("sessions", { keyPath: "id" }).createIndex("username", "username");
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error ?? new Error("could not open the browser database"));
    req.onblocked = () => reject(new Error("the browser database is open in an older tab; close other You v You tabs"));
  });
  return dbPromise;
}

function done<T>(req: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

async function store(name: string, mode: IDBTransactionMode): Promise<IDBObjectStore> {
  return (await open()).transaction(name, mode).objectStore(name);
}

export async function getPlayer(username: string): Promise<PlayerRecord | undefined> {
  return done((await store("players", "readonly")).get(username));
}

export async function putPlayer(p: PlayerRecord): Promise<void> {
  await done((await store("players", "readwrite")).put(p));
}

export async function monthsFor(username: string): Promise<MonthRecord[]> {
  return done((await store("months", "readonly")).index("username").getAll(username));
}

export async function putMonth(m: MonthRecord): Promise<void> {
  await done((await store("months", "readwrite")).put(m));
}

export async function deleteMonth(key: string): Promise<void> {
  await done((await store("months", "readwrite")).delete(key));
}

export async function sessionsFor(username: string): Promise<SessionRecord[]> {
  return done((await store("sessions", "readonly")).index("username").getAll(username));
}

export async function getSession(id: string): Promise<SessionRecord | undefined> {
  return done((await store("sessions", "readonly")).get(id));
}

export async function putSession(s: SessionRecord): Promise<void> {
  await done((await store("sessions", "readwrite")).put(s));
}

/** Asks the browser not to evict our data under storage pressure. Best effort. */
export async function requestPersistence(): Promise<void> {
  try {
    await navigator.storage?.persist?.();
  } catch {
    // not supported; data is still kept unless the browser needs the space
  }
}

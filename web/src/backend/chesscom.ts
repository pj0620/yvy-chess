// chess.com's public API, called straight from the browser (it allows cross-origin requests).
// Requests are made one at a time and retried with backoff when chess.com throttles.

const BASE = "https://api.chess.com/pub";

async function get(url: string, init: RequestInit = {}): Promise<Response | null> {
  for (let attempt = 1; ; attempt++) {
    const res = await fetch(url, { headers: { accept: "application/json" }, ...init });
    if (res.status === 404) return null;
    if ((res.status === 429 || res.status >= 500) && attempt < 5) {
      const retryAfter = Number(res.headers.get("retry-after"));
      const wait = Number.isFinite(retryAfter) && retryAfter > 0 ? retryAfter : 2 ** attempt;
      await new Promise((r) => setTimeout(r, Math.min(wait, 60) * 1000));
      continue;
    }
    if (!res.ok) throw new Error(`chess.com returned HTTP ${res.status} for ${url}`);
    return res;
  }
}

export interface ChessComPlayer {
  player_id: number | null;
  joined: number | null;
}

export async function player(username: string): Promise<ChessComPlayer | null> {
  const res = await get(`${BASE}/player/${encodeURIComponent(username)}`);
  if (!res) return null;
  const body = (await res.json()) as { player_id?: number; joined?: number };
  return { player_id: body.player_id ?? null, joined: body.joined ? body.joined * 1000 : null };
}

/** `{bullet, blitz, rapid, daily}` current ratings, where the player has them. */
export async function ratings(username: string): Promise<Record<string, number>> {
  const res = await get(`${BASE}/player/${encodeURIComponent(username)}/stats`);
  if (!res) return {};
  const stats = (await res.json()) as Record<string, { last?: { rating?: number } }>;
  const out: Record<string, number> = {};
  for (const [key, name] of [
    ["chess_bullet", "bullet"],
    ["chess_blitz", "blitz"],
    ["chess_rapid", "rapid"],
    ["chess_daily", "daily"],
  ] as const) {
    const r = stats[key]?.last?.rating;
    if (typeof r === "number") out[name] = r;
  }
  return out;
}

/** Monthly archive URLs, oldest first. */
export async function archives(username: string): Promise<string[]> {
  const res = await get(`${BASE}/player/${encodeURIComponent(username)}/games/archives`);
  if (!res) return [];
  return ((await res.json()) as { archives: string[] }).archives ?? [];
}

/** The raw JSON text of one month, handed to the Rust backend as is. */
export async function monthText(url: string, current: boolean): Promise<string> {
  // The current month changes as games are played, so revalidate it; past months are final.
  const res = await get(url, current ? { cache: "no-cache" } : {});
  return res ? res.text() : '{"games":[]}';
}

/** "2025-06" for an archive URL ending in /games/2025/06. */
export function archiveMonth(url: string): string | null {
  const m = url.match(/\/games\/(\d{4})\/(\d{2})\/?$/);
  return m ? `${m[1]}-${m[2]}` : null;
}

export function currentMonth(now = new Date()): string {
  return `${now.getUTCFullYear()}-${String(now.getUTCMonth() + 1).padStart(2, "0")}`;
}

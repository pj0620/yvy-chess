// Stockfish in a Web Worker, spoken to over UCI. The Rust backend decides what to ask and what
// to do with the answer; this module only runs searches. One search runs at a time.

export interface EngineRequest {
  session_id: string;
  fen: string;
  movetime_ms: number;
  elo: number;
  uci_elo: number | null;
  skill_level: number | null;
  candidates: string[];
}

export interface EngineLine {
  uci: string;
  depth?: number | null;
  score_cp?: number | null;
  score_mate?: number | null;
}

export interface EngineResult {
  fen: string;
  best?: EngineLine;
  lines: EngineLine[];
  error?: string;
}

export interface EngineInfo {
  name: string;
  elo_min: number;
  elo_max: number;
}

/** The part of the Worker interface we use, so tests can supply a fake engine. */
export interface WorkerLike {
  postMessage(message: string): void;
  onmessage: ((event: { data: unknown }) => void) | null;
  onerror: ((event: unknown) => void) | null;
  terminate(): void;
}

export const ENGINE_FILE = "engine/stockfish-19-lite-single.js";

export function browserWorker(): WorkerLike {
  return new Worker(new URL(ENGINE_FILE, document.baseURI)) as unknown as WorkerLike;
}

/** Parses one UCI `info` line into a line of a (multi-PV) search, or null without a PV. */
export function parseInfo(line: string): { index: number; line: EngineLine } | null {
  const t = line.trim().split(/\s+/);
  if (t[0] !== "info") return null;
  let index = 1;
  const out: EngineLine = { uci: "" };
  for (let i = 1; i < t.length; i++) {
    switch (t[i]) {
      case "multipv":
        index = Number(t[i + 1]) || 1;
        break;
      case "depth":
        out.depth = Number(t[i + 1]);
        break;
      case "score":
        if (t[i + 1] === "cp") {
          out.score_cp = Number(t[i + 2]);
          out.score_mate = null;
        } else if (t[i + 1] === "mate") {
          out.score_mate = Number(t[i + 2]);
          out.score_cp = null;
        }
        break;
      case "pv":
        out.uci = t[i + 1] ?? "";
        i = t.length;
        break;
    }
  }
  return out.uci ? { index, line: out } : null;
}

export class Engine {
  private worker: WorkerLike | null = null;
  private listeners = new Set<(line: string) => void>();
  private starting: Promise<EngineInfo> | null = null;
  private queue: Promise<unknown> = Promise.resolve();

  constructor(
    private readonly spawn: () => WorkerLike = browserWorker,
    private readonly timeoutMs = 20_000,
  ) {}

  private send(cmd: string) {
    this.worker?.postMessage(cmd);
  }

  /** Resolves with the first line accepted by `pick`, collecting earlier lines via `seen`. */
  private until<T>(pick: (line: string) => T | undefined, timeoutMs: number, seen?: (line: string) => void): Promise<T> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.listeners.delete(listener);
        reject(new Error("Stockfish did not answer in time"));
      }, timeoutMs);
      const listener = (line: string) => {
        const value = pick(line);
        if (value !== undefined) {
          clearTimeout(timer);
          this.listeners.delete(listener);
          resolve(value);
        } else {
          seen?.(line);
        }
      };
      this.listeners.add(listener);
    });
  }

  /** Starts the worker once and completes the UCI handshake. */
  info(): Promise<EngineInfo> {
    if (this.starting) return this.starting;
    this.starting = (async () => {
      const worker = this.spawn();
      this.worker = worker;
      worker.onmessage = (e) => {
        for (const line of String(e.data).split("\n")) {
          if (line.trim()) for (const l of [...this.listeners]) l(line);
        }
      };
      const failed = new Promise<never>((_, reject) => {
        worker.onerror = () => reject(new Error("Stockfish could not be loaded in this browser"));
      });
      const info: EngineInfo = { name: "Stockfish", elo_min: 1320, elo_max: 3190 };
      const handshake = this.until(
        (l) => (l.trim() === "uciok" ? true : undefined),
        this.timeoutMs,
        (l) => {
          if (l.startsWith("id name ")) info.name = l.slice(8).trim();
          const m = l.match(/^option name UCI_Elo type spin .*min (\d+) max (\d+)/);
          if (m) {
            info.elo_min = Number(m[1]);
            info.elo_max = Number(m[2]);
          }
        },
      );
      this.send("uci");
      await Promise.race([handshake, failed]);
      const ready = this.until((l) => (l.trim() === "readyok" ? true : undefined), this.timeoutMs);
      this.send("isready");
      await Promise.race([ready, failed]);
      return info;
    })();
    this.starting.catch(() => {
      this.worker?.terminate();
      this.worker = null;
      this.starting = null;
    });
    return this.starting;
  }

  private async search(fen: string, movetimeMs: number, searchmoves?: string[]): Promise<{ bestmove: string; lines: EngineLine[] }> {
    const multipv = searchmoves?.length ? searchmoves.length : 1;
    const slots: (EngineLine | undefined)[] = [];
    const done = this.until(
      (l) => {
        const t = l.trim().split(/\s+/);
        return t[0] === "bestmove" ? (t[1] ?? "(none)") : undefined;
      },
      movetimeMs + this.timeoutMs,
      (l) => {
        const info = parseInfo(l);
        if (info && info.index >= 1 && info.index <= multipv) slots[info.index - 1] = info.line;
      },
    );
    this.send(`setoption name MultiPV value ${multipv}`);
    this.send(`position fen ${fen}`);
    this.send(`go movetime ${movetimeMs}${searchmoves?.length ? ` searchmoves ${searchmoves.join(" ")}` : ""}`);
    const bestmove = await done;
    if (bestmove === "(none)" || bestmove === "0000") throw new Error("the engine has no legal move");
    return { bestmove, lines: slots.filter((s): s is EngineLine => !!s) };
  }

  /**
   * Best move at the requested strength, plus equal-budget scores for the steering candidates
   * when the best move is not one of them. Never throws: failures come back as `error`.
   */
  evaluate(req: EngineRequest): Promise<EngineResult> {
    const run = async (): Promise<EngineResult> => {
      try {
        await this.info();
        if (req.uci_elo != null) {
          this.send("setoption name Skill Level value 20");
          this.send("setoption name UCI_LimitStrength value true");
          this.send(`setoption name UCI_Elo value ${req.uci_elo}`);
        } else {
          this.send("setoption name UCI_LimitStrength value false");
          this.send(`setoption name Skill Level value ${req.skill_level ?? 20}`);
        }
        const top = await this.search(req.fen, req.movetime_ms);
        const first = top.lines[0];
        const best: EngineLine = {
          uci: top.bestmove,
          depth: first?.depth ?? null,
          score_cp: first?.score_cp ?? null,
          score_mate: first?.score_mate ?? null,
        };
        let lines: EngineLine[] = [];
        if (req.candidates.length > 0 && !req.candidates.includes(best.uci)) {
          lines = (await this.search(req.fen, req.movetime_ms, req.candidates)).lines;
        }
        return { fen: req.fen, best, lines };
      } catch (e) {
        return { fen: req.fen, lines: [], error: (e as Error).message };
      }
    };
    const next = this.queue.then(run, run);
    this.queue = next;
    return next;
  }

  terminate() {
    this.worker?.terminate();
    this.worker = null;
    this.starting = null;
  }
}

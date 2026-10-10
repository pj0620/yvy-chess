# You v You Chess: Build Plan

## Status (2026-10-10): now a static site, Rust in WebAssembly

The app no longer has a server or a database. The Rust logic moved into `crates/yvy-app` and
runs in the browser as WebAssembly (`crates/yvy-wasm`); imports, games played here and the
index live in the browser's IndexedDB; Stockfish 19 (lite, single-threaded WebAssembly build)
runs in a Web Worker. The site deploys to GitHub Pages (`.github/workflows/pages.yml`).

What changed relative to the plan below:
- Postgres, axum, Docker Compose and the native Stockfish process are gone. Sections 3 to 9
  describe that server architecture and are kept as history.
- Positions and own-piece setups are keyed by 64-bit hashes instead of EPD strings, and moves are
  packed into 16 bits, so a large history fits in browser memory. Imported decisions sit in a
  sorted array searched by binary search; games played here sit in a small mutable overlay.
- Each imported chess.com month is stored as one compact binary blob produced by Rust.
- Engine work is a round trip: Rust returns an engine request (position, strength, steering
  candidates), the page runs Stockfish, and Rust chooses the move from the result.
- Games played here can be saved and restored as a backup file, since they exist only in the
  browser.

Everything in section 2 (the model, steering, switch mode, memory) carries over unchanged.

## 1. What we're building

Enter a chess.com username. The app downloads every game that account has played, replays them, and builds an index: for every position in which *you* were to move, which move you actually played and how often. You then play against that index. Whenever the board reaches a position you have faced before, the opponent answers with one of your own historical moves, chosen at random in proportion to how often you played it (two games, two different moves: 50/50). When the board is in a position you have never faced, Stockfish fills the gap at a strength you choose. Every opponent move is labelled with its origin: a human icon when it came from your history, an engine icon when Stockfish had to fill in.

Backend: Rust. Frontend: Bun as runtime, bundler and package manager, with React + TypeScript. Database: Postgres running in Docker, so nothing depends on a database installed on the laptop, and the same Compose file later runs the whole app.

## 2. How the "you" model works

### Position key, not move tree

Index by *position*, not by move sequence. The key is the EPD (a FEN without the two move counters): piece placement, side to move, castling rights, en-passant square. Side to move is part of the key, so White-you and Black-you are naturally separate models, and transpositions (the same position reached by a different move order) merge automatically. That gives strictly more coverage than a literal tree of move sequences, and it is exactly what "find a time you faced board X" means.

En passant normalization: only write the ep square when an ep capture is actually legal (shakmaty's `EnPassantMode::Legal`). Otherwise identical positions get different keys.

### Building the index

For each game where the account played, as White or Black:
- Skip anything with `rules != "chess"` (chess960, bughouse, etc.). Chess960 could get its own key space later.
- Replay the PGN with shakmaty. At each ply where side-to-move equals your color, insert one row: `(game_id, ply, username, epd, uci_move)`.
- Nothing is recorded for your opponents' moves. The model simulates your decisions only.

Scale: a heavy account (10k games, ~40 of your moves each) is about 400k rows. Trivial for Postgres; import time is dominated by the network.

### Choosing a reply

```sql
SELECT pm.uci, COUNT(*) AS n, array_agg(pm.game_id) AS game_ids
FROM position_moves pm
JOIN games g ON g.id = pm.game_id
WHERE pm.username = $1 AND pm.epd = $2
  AND ($3::text[]      IS NULL OR g.time_class = ANY($3))
  AND ($4::timestamptz IS NULL OR g.end_time >= $4)
  AND ($5::timestamptz IS NULL OR g.end_time <= $5)
  AND ($6::boolean     IS NULL OR g.rated = $6)
GROUP BY pm.uci
ORDER BY n DESC;
```

```
if rows is non-empty:  sample a uci with P(move) = n / sum(n)     -> source = "history"
else:                  ask Stockfish at the configured strength  -> source = "engine"
```

Filters (time class, date range, rated only) are applied in the query, so changing them needs no re-indexing. Every reply carries its provenance and, for history moves, the full candidate distribution so the UI can say "you've been here 12 times: c5 x8, e5 x4" and link to the source games. A "most frequent" mode (argmax instead of sampling) is a one-line switch and worth exposing.

### Steering back toward your games (implemented)

Exact-position matching runs out after the opening, and a plain engine move then drifts
further away. So a gap is filled in two steps. First, every legal model move is scored by how
many of your past games pass through a position one opponent reply later (all replies are
generated and looked up in one query); that catches move-order transpositions, which is where
coverage can actually be recovered. Second, Stockfish gives its best move and evaluation, the
steering candidates are scored with a restricted search of the same budget, and the candidate
with the most games ahead that stays within a per-game tolerance of the best evaluation is
played (off, 50, 100 or 250 centipawns). A second, looser signal covers the common case where
an irreversible move (a pawn push) makes exact return impossible: each decision also records
the placement of the player's own pieces after the move (`position_moves.setup`), and a
candidate scores by how many past games had exactly that setup, so the model keeps reaching
for the development patterns the player actually uses. Exact transpositions rank first, then
familiar setups, then evaluation. Steered moves carry their own badge and record what was
given up. On the human's turn the API also lists the moves after which the model is back in its
history, so the player can steer too. Everything reads the existing index; no re-import needed.

### Switch-sides gap mode (implemented)

A per-session `gap_mode`: `stockfish` (engine plus steering) or `switch`. In switch mode a gap
hands the model's side to the human: `sessions.current_color` flips, a `{ply, color}` entry is
appended to `sessions.switches`, and no model move is played. The model then answers as the
human's former side from that side's history, and switches again when that runs out. Undo
restores the side that made the taken-back move and drops later switches. Memory records the
human's moves for whichever side they played, so the holes get filled where they are.

### Games played here feed the model (implemented)

Each session is stored as a game of its own (`games.id = yvy:session:<uuid>`, time class `yvy`)
and the human's moves become `position_moves` rows as they are played, with undo removing them
again. The model's moves are never recorded. Everything downstream (sampling, filters,
steering, returning moves) works unchanged, and the time-class filter can exclude app games.

### What coverage to expect

Nearly every chess position past roughly move 12 is unique within one player's history. So the human icon will dominate the opening and reappear sporadically later (transpositions, repeated lines, forced sequences), and Stockfish will carry most middlegames and endgames. The UI should make the handoff visible rather than hide it. That handoff is the interesting part of the product.

## 3. Architecture

```
 Browser: React SPA built by Bun
   |  JSON over HTTP, plus SSE for import progress
   v
 yvy-server (Rust, axum)                       docker compose
   |- chesscom : reqwest client for api.chess.com/pub      +---------------------+
   |- import   : PGN -> position rows, bulk inserts        |  db: postgres:17    |
   |- model    : position lookup + weighted sampling  <--> |  volume: pgdata     |
   |- engine   : UCI driver around a Stockfish process     |  port 5432          |
   |- game     : authoritative state, legality, mate/draw  +---------------------+
   '- db       : sqlx PgPool, embedded migrations

 stockfish : child process speaking UCI over stdin/stdout
             (host binary in dev, baked into the app image in the full profile)
```

Two ways to run it, from one Compose file:
- **dev**: `docker compose up -d db`, then `cargo run` and `bun --hot` on the host for fast iteration. Stockfish from Homebrew.
- **full**: `docker compose --profile full up --build` runs Postgres plus the app image, which contains the Rust server, the built web bundle and Stockfish. This is also the deployment shape.

Key choices and why:

- **axum + tokio** for HTTP. The standard async Rust stack. Plain REST is enough because play is turn-based; SSE covers the one long-running thing (import).
- **shakmaty + pgn-reader** (same author as python-chess and the lichess backend). Fast, correct move generation, FEN/EPD/SAN/UCI conversions, and a streaming PGN parser that tolerates chess.com's `{[%clk ...]}` comments.
- **Postgres via sqlx.** Postgres is the boring, correct default: JSONB for the filter and detail blobs, arrays for `array_agg` and `= ANY($n)` filters, `UNNEST` bulk inserts, a multi-arch official image, and it scales to a hosted multi-user deployment without changes. sqlx gives an async pool, embedded migrations and `#[sqlx::test]` for isolated per-test databases. Start with runtime-checked `sqlx::query_as` so builds never need a database; switch to the compile-time `query!` macros with offline data later if you want that safety net.
- **Server-owned game state.** Rust validates every move and persists every game, so a page refresh never loses a game and the frontend contains no chess logic: it renders state and sends moves. Legal moves are returned with every state so the board can highlight targets without a browser chess library.
- **Stockfish as a subprocess**, not WASM in the browser. Keeps every decision in the backend, and the native binary is far stronger per millisecond. See section 7 for where the binary comes from.
- **reqwest and sqlx with rustls**, not OpenSSL, so the runtime image needs no libssl and the Rust builder and runtime images do not have to share a Debian release.
- **Bun**: `bun init --react` scaffold, Bun's dev server with HMR, `bun build` for production. `react-chessboard` for the board, TanStack Query for server state, react-router for three routes. The production bundle is served by the Rust binary so the app is one process.

## 4. Data model (Postgres)

Migrations live in `crates/yvy-server/migrations/` and run at server startup via `sqlx::migrate!`, so the `app` container migrates itself.

```sql
CREATE TABLE players (
  username        TEXT PRIMARY KEY,             -- lowercased; chess.com usernames are case-insensitive
  player_id       BIGINT,
  joined          TIMESTAMPTZ,
  last_import_at  TIMESTAMPTZ,
  ratings         JSONB                         -- from /stats: bullet, blitz, rapid, daily
);

CREATE TABLE archives (
  username    TEXT NOT NULL REFERENCES players(username),
  year        INT  NOT NULL,
  month       INT  NOT NULL,
  etag        TEXT,
  fetched_at  TIMESTAMPTZ,
  game_count  INT,
  complete    BOOLEAN NOT NULL DEFAULT false,   -- past months never change
  PRIMARY KEY (username, year, month)
);

CREATE TABLE games (
  id               TEXT PRIMARY KEY,            -- the chess.com game url
  username         TEXT NOT NULL REFERENCES players(username),
  color            TEXT NOT NULL CHECK (color IN ('white','black')),
  time_class       TEXT NOT NULL,               -- bullet | blitz | rapid | daily
  time_control     TEXT,
  rules            TEXT NOT NULL,               -- only 'chess' is indexed
  rated            BOOLEAN NOT NULL,
  end_time         TIMESTAMPTZ NOT NULL,        -- to_timestamp(end_time) from the API
  result           TEXT NOT NULL,               -- raw chess.com code for *you*: win, resigned, timeout, agreed, ...
  opponent         TEXT,
  player_rating    INT,
  opponent_rating  INT,
  eco              TEXT,
  ply_count        INT,
  pgn              TEXT NOT NULL                -- kept so we can re-index with new logic without re-downloading
);
CREATE INDEX games_filters ON games (username, time_class, end_time);

CREATE TABLE position_moves (
  game_id   TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
  ply       INT  NOT NULL,
  username  TEXT NOT NULL,                      -- denormalized from games so the lookup needs no join
  epd       TEXT NOT NULL,
  uci       TEXT NOT NULL,
  san       TEXT NOT NULL,                      -- as written in the game, for display
  setup     TEXT,                               -- own-piece placement after the move, for steering
  PRIMARY KEY (game_id, ply)
);
CREATE INDEX position_moves_lookup ON position_moves (username, epd);

CREATE TABLE sessions (
  id          UUID PRIMARY KEY,
  username    TEXT NOT NULL REFERENCES players(username),
  user_color  TEXT NOT NULL CHECK (user_color IN ('white','black')),
  engine_elo  INT  NOT NULL,
  mode        TEXT NOT NULL CHECK (mode IN ('sampled','most_frequent')),
  filters     JSONB NOT NULL DEFAULT '{}',
  status      TEXT NOT NULL,                    -- playing | checkmate | stalemate | draw | resigned
  result      TEXT,
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE session_moves (
  session_id  UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  ply         INT  NOT NULL,
  uci         TEXT NOT NULL,
  san         TEXT NOT NULL,
  source      TEXT NOT NULL CHECK (source IN ('player','history','engine')),
  detail      JSONB,                            -- candidate distribution, or engine depth/eval
  PRIMARY KEY (session_id, ply)
);
```

Bulk inserts use the `UNNEST` pattern, one statement per batch of a few thousand rows:

```sql
INSERT INTO position_moves (game_id, ply, username, epd, uci)
SELECT * FROM UNNEST($1::text[], $2::int[], $3::text[], $4::text[], $5::text[])
ON CONFLICT DO NOTHING;
```

## 5. HTTP API

| Method | Path | Purpose |
|---|---|---|
| POST | `/api/players/{u}/import` | Start or resume an import. 202 with a job id. Idempotent while running. |
| GET | `/api/players/{u}/import/events` | SSE stream: `{phase, archives_done, archives_total, games, positions, errors}` |
| GET | `/api/players/{u}` | Summary: games by color and time class, positions indexed, ratings, last import |
| GET | `/api/players/{u}/positions?epd=...&filters...` | Candidate distribution for one position. Powers the "you've been here" panel. |
| POST | `/api/sessions` | `{username, user_color, engine: {elo}, mode, filters}`. Returns state. If you are Black, the model moves first. |
| GET | `/api/sessions/{id}` | Full state, shape below |
| POST | `/api/sessions/{id}/moves` | `{uci}`. Applies your move, computes the reply, returns state. |
| POST | `/api/sessions/{id}/undo` | Takes back your last move and the reply |
| POST | `/api/sessions/{id}/resign` | |
| GET | `/api/sessions/{id}/pgn` | Export with provenance comments |
| GET | `/api/engine` | Is Stockfish available? version, path, supported Elo range |
| GET | `/api/health` | Pings the database and the engine; used by the Compose healthcheck |

Game state payload:

```json
{
  "id": "…", "fen": "…", "turn": "black", "status": "playing",
  "user_color": "white", "engine_elo": 1500, "mode": "sampled",
  "legal_moves": ["e2e4", "g1f3"],
  "moves": [
    {"ply": 1, "san": "e4",  "uci": "e2e4", "source": "player"},
    {"ply": 2, "san": "c5",  "uci": "c7c5", "source": "history",
     "detail": {"games": 12, "candidates": [
        {"uci": "c7c5", "san": "c5", "count": 8, "game_ids": ["…"]},
        {"uci": "e7e5", "san": "e5", "count": 4, "game_ids": ["…"]}]}},
    {"ply": 3, "san": "Nf3", "uci": "g1f3", "source": "player"},
    {"ply": 4, "san": "d6",  "uci": "d7d6", "source": "engine",
     "detail": {"elo": 1500, "depth": 10, "eval_cp": 25, "movetime_ms": 300}}
  ],
  "coverage": {"in_history": true, "games": 3}
}
```

`coverage` describes the position currently on the board, so the UI can show whether "you" are about to answer before the player even moves.

## 6. Import pipeline (chess.com specifics)

Field names below were checked against the live API on 2026-10-08. Rate-limit behaviour is from chess.com's published guidance.

1. `GET /pub/player/{u}`: 404 means unknown user.
2. `GET /pub/player/{u}/stats`: `chess_blitz.last.rating`, `chess_rapid.last.rating`, etc. Used as the default Stockfish Elo for the most-played selected time class.
3. `GET /pub/player/{u}/games/archives`: list of monthly URLs. Hikaru has 154 months.
4. For each month, **serially** (chess.com rate-limits parallel requests; back off on 429): `GET /pub/player/{u}/games/{YYYY}/{MM}` returns `games[]` with `url, pgn, time_class, time_control, rules, rated, end_time, eco, uuid, white{username, rating, result}, black{...}`. A busy month is around 500 games and a few MB.
5. Per month, in one transaction: upsert `games`, replay the PGNs, bulk-insert `position_moves` with `UNNEST`. Store the month's `ETag`. The API returns `etag` and `last-modified`, so re-imports send `If-None-Match` and skip unchanged months. Past months are effectively immutable; only the current month changes.
6. Always send a `User-Agent` with contact info, which chess.com asks for: `yvy-chess/0.1 (+your-email)`.

Failure handling: a PGN that does not parse, or a game with zero moves, is logged, counted in the import report and skipped. Nothing aborts the import. The import runs as a background tokio task, so closing the browser does not stop it.

## 7. Engine integration

The engine is **Stockfish**, and it is the only engine in the system. The "you" model is a lookup table, not an engine; Stockfish is consulted only when the position is missing from your history. Any UCI engine would work, which matters for one later idea below.

Where the binary comes from:
- **dev**: `brew install stockfish` on the host; `STOCKFISH_PATH` defaults to whatever `stockfish` resolves to on PATH.
- **full profile and deployment**: the app image is based on `debian:trixie-slim`, whose `stockfish` package is Stockfish 17, installed at `/usr/games/stockfish`. The distro package is multi-arch, so the same Dockerfile builds on this Apple Silicon machine (linux/arm64) and on x86-64 servers. The official Stockfish release binaries do not ship a plain linux-arm64 build, which is why the package is preferred. If a specific version is ever needed, add a builder stage that compiles a pinned tag from source.

A hand-written UCI driver, roughly 150 lines over `tokio::process`:

```
uci
setoption name UCI_LimitStrength value true
setoption name UCI_Elo value 1500
isready
ucinewgame
position fen <fen>
go movetime 300
  ... bestmove d7d6 ponder ...
```

One long-lived process behind a `tokio::sync::Mutex`, respawned on crash. Stockfish's `UCI_Elo` range is about 1320 to 3190; for weaker targets fall back to `Skill Level` 0 to 20. User-facing settings: target Elo (default: your chess.com rating for the selected time class) and think time. If Stockfish is missing, `/api/engine` says so and the UI shows the install command. A dev-only "random legal move" fallback keeps the loop usable without it.

Later alternative worth knowing about: **Maia** is a neural network trained to predict the move a human of a given rating would play, and it runs as a weights file inside the lc0 engine, which also speaks UCI. Stockfish at `UCI_Elo 1500` plays weak but engine-like moves; Maia 1500 plays human-like 1500 moves, which fits the "you" illusion better. Because the driver is plain UCI, it would be a config swap plus a bigger image.

## 8. Frontend

Routes:
- `/`: enter a username. Validates via `/api/players/{u}` and kicks off the import if the account has never been imported.
- `/p/:username`: import progress live over SSE, then summary stats and the "New game" form: your color, time classes to include, date range, Stockfish Elo prefilled from your rating, sampled vs most-frequent mode.
- `/g/:id`: the board.

Board screen components:
- `Board` (react-chessboard): drag and drop restricted to `legal_moves`, promotion picker, oriented to your color, last-move highlight.
- `ProvenanceBadge`: shown beside the board after each reply and inline in the move list. Human icon with "From your games, 8 of 12" or engine icon with "Stockfish 1500". Hover or click opens the candidate distribution as a small bar chart with links to the source games on chess.com.
- `CoverageIndicator`: whether the current position exists in your history.
- `MoveList`, `GameControls` (undo, resign, new game, flip, export PGN), `GameOverModal`.

State: TanStack Query for everything server-side. Optimistically apply your own move, show a "thinking" shimmer until the reply arrives. No chess library in the browser; legality and SAN come from the server.

## 9. Repo layout, Docker and dev workflow

```
yvy_chess/
|- Cargo.toml                  # workspace
|- rust-toolchain.toml         # channel = "stable" (this machine's default toolchain is `esp`)
|- docker-compose.yml          # db service always; app service under profile "full"
|- Dockerfile                  # multi-stage: bun build, cargo build, trixie-slim + stockfish
|- .env.example                # DATABASE_URL, STOCKFISH_PATH, YVY_BIND, YVY_CONTACT
|- crates/
|  |- yvy-core/                # pure: position keys, PGN -> positions, sampling. No IO. Heavily unit-tested.
|  '- yvy-server/              # axum app: chesscom, import, engine, game, db, plus a CLI (`yvy import <user>`)
|     '- migrations/           # sqlx migrations, embedded into the binary
|- web/                        # Bun + React + TS (bun init --react)
|  |- package.json
|  '- src/
|- Makefile                    # make dev / make test / make up / make build (just is not installed here)
'- PLAN.md
```

`docker-compose.yml`:

```yaml
services:
  db:
    image: postgres:17
    restart: unless-stopped
    environment:
      POSTGRES_USER: yvy
      POSTGRES_PASSWORD: yvy
      POSTGRES_DB: yvy
    ports:
      - "5432:5432"
    volumes:
      - pgdata:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U yvy -d yvy"]
      interval: 5s
      timeout: 3s
      retries: 10

  app:
    profiles: ["full"]
    build: .
    depends_on:
      db:
        condition: service_healthy
    environment:
      DATABASE_URL: postgres://yvy:yvy@db:5432/yvy
      STOCKFISH_PATH: /usr/games/stockfish
      YVY_BIND: 0.0.0.0:8080
      YVY_WEB_DIR: /srv/web
      YVY_CONTACT: you@example.com
    ports:
      - "8080:8080"

volumes:
  pgdata:
```

`Dockerfile` (sketch):

```dockerfile
FROM oven/bun:1 AS web
WORKDIR /web
COPY web/package.json web/bun.lock ./
RUN bun install --frozen-lockfile
COPY web/ .
RUN bun run build                                   # writes /web/dist

FROM rust:1-bookworm AS server
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates/ crates/
RUN cargo build --release -p yvy-server             # add cargo-chef if rebuilds get slow

FROM debian:trixie-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends stockfish ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=server /src/target/release/yvy-server /usr/local/bin/yvy-server
COPY --from=web /web/dist /srv/web
ENV STOCKFISH_PATH=/usr/games/stockfish YVY_WEB_DIR=/srv/web YVY_BIND=0.0.0.0:8080
EXPOSE 8080
CMD ["yvy-server"]
```

Dev loop (`make dev`): checks that the Docker daemon is up, `docker compose up -d db --wait`, then the server on `YVY_BIND` (migrations run at startup) and `bun --hot` on :3000 with `/api/*` proxied to the server from `Bun.serve` routes; the Bun dev server reads the repo `.env` to find the API. Full stack: `make up` runs `docker compose --profile full up --build`. Host ports are overridable with `YVY_DB_PORT` and `YVY_APP_PORT` because 5432 and 8080 were both taken on the development machine. The web bundle is served by the Rust binary from `YVY_WEB_DIR` through `tower-http` `ServeDir` with an SPA fallback.

Config via env, with `.env.example` as the template:
- `DATABASE_URL` (dev default `postgres://yvy:yvy@localhost:5432/yvy`)
- `STOCKFISH_PATH` (default: resolve `stockfish` on PATH)
- `YVY_BIND` (default `127.0.0.1:8080`)
- `YVY_WEB_DIR` (unset in dev; Bun serves the SPA)
- `YVY_CONTACT` for the chess.com User-Agent

Tests:
- `yvy-core`: replay a fixture PGN and assert that every one of the player's positions maps to the move actually played; EPD normalization cases (ep legal vs not, castling rights); weighted sampling distribution test with a seeded RNG. No database.
- `yvy-server`: `#[sqlx::test]` integration tests, each in its own freshly migrated database on the Compose Postgres; import against a recorded archive JSON fixture, no network; API tests with `tower::ServiceExt::oneshot`; engine driver test skipped when Stockfish is absent.
- CI: GitHub Actions with a `postgres:17` service container for `cargo test`, plus `bun test`, plus a `docker build` of the app image.
- `web`: `bun test` for ProvenanceBadge and MoveList rendering; one Playwright smoke test later.

## 10. Milestones

**M0, scaffold (half a day).** Cargo workspace plus `rust-toolchain.toml`, Bun React app, `docker-compose.yml` with the `db` service, first migration, sqlx pool and `/api/health`, Makefile, `.env.example`, replace the Python `.gitignore` (add `target/`, `node_modules/`, `web/dist/`, `.env`), CI running `cargo test` against a Postgres service and `bun test`. `brew install stockfish`.

**M1, import and index (1 to 2 days).** chess.com client, PGN to `position_moves` with `UNNEST` bulk inserts, `yvy import <user>` and `yvy lookup <user> "<fen>"` CLI commands. Done when your own account imports end to end and looking up a position from one of your games returns the move you played.

**M2, play engine (1 to 2 days).** Session state machine, legal-move validation, history sampling, Stockfish driver and fallback, provenance on every reply, mate and draw detection (checkmate, stalemate, insufficient material, threefold, 50-move), undo. Done when you can play a full game with curl and the move log shows a plausible history-to-engine handoff.

**M3, UI (2 to 3 days).** Username, import progress, settings, board. ProvenanceBadge with human and engine icons, candidate distribution popover, game over, new game. Done when someone else can enter a username and play without reading docs.

**M4, ship it (1 day, then ongoing).** `Dockerfile` and the `full` Compose profile with Stockfish baked in, `docker build` in CI, re-sync button using ETags, filters UI, coverage indicator, "what you played here" hint toggle for your own moves, PGN export with provenance, PGN file upload as an alternative to chess.com import.

## 11. Risks and open questions

- **Coverage is thinner than people expect** past the opening. Mitigations so far: loud provenance, "N games reached this position", and steering (section 2). A clearly labelled "fuzzy" mode (ignore castling rights, or key on pawn structure plus material) and deeper steering (three plies, or along the engine's principal variations) are the next levers.
- **Huge accounts** (50k+ games): import takes minutes. Per-month ETags make it resumable and it must never block the UI.
- **Stockfish Elo is not chess.com Elo.** The default is a heuristic; the slider is the real control. Maia (section 7) is the better long-term answer.
- **chess.com API changes or limits.** Serial requests, User-Agent, backoff. The PGN upload path (M4) also decouples tests from the network and lets lichess users in.
- **Docker has to be running for dev.** Docker 27 and Compose v2 are installed here but the daemon was not running when checked. `just dev` should fail fast with a clear message if the socket is missing.
- **sqlx compile-time checking** needs a live database or committed offline query data at build time. Start with runtime-checked queries; adopt the macros plus `cargo sqlx prepare` only if the extra safety is wanted.
- **Toolchain on this machine.** The default rustup toolchain is `esp` (1.77 nightly, x86_64). Current crates need recent stable; `rust-toolchain.toml` fixes that per repo. Installing `stable-aarch64-apple-darwin` would give native builds on this Apple Silicon machine.

## 12. Later ideas

- Maia via lc0 as the gap filler, selected per session alongside Stockfish.
- "You resigned here": if your history from a position is mostly resignations or timeouts, the model resigns.
- Era personas: "2019 you" vs "2026 you" as named date-range presets.
- Result-weighted or recency-weighted sampling.
- Lichess import. Their API streams NDJSON with PGN; the same index applies.
- Spectator mode: White-you vs Black-you, Stockfish filling gaps on both sides.
- Opening coverage explorer: a tree view of where your history runs out.

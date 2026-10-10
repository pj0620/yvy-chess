# You v You Chess

Play against a model of your own chess.com history.

Enter a chess.com username. The server downloads every game that account has played, replays
them, and indexes every decision the player made: the position they faced and the move they
chose. Then you play a game against that index. Whenever the board reaches a position you have
faced before, your opponent answers with one of your own moves, chosen at random in proportion
to how often you played it. When the position is one you have never faced, Stockfish fills the
gap at a strength you choose. Every reply is labelled: a human icon when it came from your games,
an engine icon when Stockfish had to fill in.

The design is in [PLAN.md](PLAN.md).

## Stack

- **Backend**: Rust. axum for HTTP, sqlx for Postgres, shakmaty and pgn-reader for chess,
  a small UCI driver for Stockfish.
- **Database**: Postgres 17 in Docker Compose. Nothing depends on a database installed locally.
- **Frontend**: Bun as runtime, bundler and package manager; React + TypeScript; react-chessboard.
- **Engine**: Stockfish as a child process. Homebrew's build in dev, Debian's package inside the
  Docker image.

## Prerequisites

- Docker Desktop, running.
- Rust stable (`rust-toolchain.toml` pins the channel; rustup installs it on first use).
- Bun 1.2 or newer.
- Stockfish for local development: `brew install stockfish`. The Docker image brings its own.

## Run it

```sh
cp .env.example .env     # adjust YVY_DB_PORT / YVY_BIND / YVY_APP_PORT if those ports are taken
make dev                 # Postgres in Docker, Rust API, Bun dev server with HMR on http://localhost:3000
```

Open http://localhost:3000, enter a chess.com username, watch the import, start a game.

`make dev` runs everything in the foreground. `make launch` (or `scripts/launch.sh`) starts the
same stack detached, waits until it is healthy and prints the URLs; `make status` and `make stop`
go with it. In Claude Code the same thing is the `/launch` skill.

Everything in Docker (Rust server, web bundle and Stockfish in one image, plus Postgres):

```sh
make up                  # http://localhost:8080, or YVY_APP_PORT from .env
```

## Developer commands

| Command | What it does |
|---|---|
| `make dev` | Start Postgres, the Rust server and the Bun dev server |
| `make server` / `make web` | Only one of the two servers |
| `make test` | Rust unit + Postgres integration tests, then Bun tests |
| `make build` | Production web bundle and release binary on the host |
| `make up` / `make down` | Full Compose profile (app image + Postgres) |
| `make import USER=name ARGS="--months 3"` | Import an account from the CLI |
| `make lookup USER=name FEN="..."` | Show what the player did from a position |

The CLI is also available directly:

```sh
cargo run -p yvy-server -- import <username> [--months N]
cargo run -p yvy-server -- lookup <username> "<fen>" [--time-classes blitz,rapid]
cargo run -p yvy-server -- reindex <username>      # rebuild the index from stored games
cargo run -p yvy-server -- health                   # exit 0 if the server at YVY_BIND is healthy
cargo run -p yvy-server -- serve
```

## Configuration

All settings are environment variables, read from `.env` in the repo root.

| Variable | Default | Meaning |
|---|---|---|
| `DATABASE_URL` | `postgres://yvy:yvy@localhost:5432/yvy` | Postgres connection |
| `YVY_DB_PORT` | `5432` | Host port of the Compose Postgres |
| `YVY_BIND` | `127.0.0.1:8080` | Where the Rust server listens; the Bun dev proxy follows it |
| `YVY_APP_PORT` | `8080` | Host port of the app container in the full profile |
| `STOCKFISH_PATH` | `stockfish` | Engine binary (resolved on PATH) |
| `YVY_ENGINE_MOVETIME_MS` | `300` | Think time per engine move |
| `YVY_WEB_DIR` | unset | Directory of the built web bundle; when set, the Rust binary serves the SPA |
| `YVY_CONTACT` | `unknown` | Contact string sent in the chess.com User-Agent |
| `YVY_PASSWORD` | unset | When set, every request except `/api/health` needs this password (HTTP Basic). Set it before exposing the app publicly. |
| `RUST_LOG` | `info` | Log filter |

## HTTP API

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/health` | Database and engine check |
| `GET` | `/api/engine` | Engine availability, name and Elo range |
| `GET` | `/api/players/{u}` | Import summary: games, decisions indexed, ratings |
| `POST` | `/api/players/{u}/import?months=N` | Start or resume an import (validates the user on chess.com) |
| `GET` | `/api/players/{u}/import` | Current import progress |
| `GET` | `/api/players/{u}/import/events` | Server-sent events with progress |
| `GET` | `/api/players/{u}/positions?fen=...` | What the player did from a position, with filters |
| `GET` | `/api/players/{u}/sessions` | Games played in the app, most recent first |
| `POST` | `/api/sessions` | New game: `{username, user_color, engine_elo?, mode?, filters?}` |
| `GET` | `/api/sessions/{id}` | Game state: FEN, legal moves, moves with provenance, coverage |
| `POST` | `/api/sessions/{id}/moves` | Play `{uci}`; the reply comes back in the same response |
| `POST` | `/api/sessions/{id}/undo` | Take back your last move and the reply |
| `POST` | `/api/sessions/{id}/resign` | Resign |
| `GET` | `/api/sessions/{id}/pgn` | PGN with a provenance comment after every model move |

Each model move carries `source` (`history` or `engine`) and a `detail` object: the full
candidate distribution for history moves, or Elo, depth and evaluation for Stockfish moves.

## How the model works

Positions are keyed by EPD (FEN without move counters, en-passant square only when a capture
is legal), so transpositions merge. For each game the player appears in, every ply where they
were to move becomes one row: position key, move, game. A reply is a single indexed query
grouped by move, filtered by time class, date range or rated flag at query time. If the query
returns nothing, the gap is filled with steering: every legal move is scored by how many of
your past games lie one reply ahead of it and by how many past games had the same placement
of your own pieces after it, Stockfish evaluates the best move and those candidates with the
same budget, and the strongest-steering candidate within the game's
tolerance (off, light, normal, strong) is played. Steered moves get their own badge. When it is
your move, the position card lists the moves after which the model is back in your games. If
Stockfish is not installed, the top steering candidate or a random legal move is played and
labelled as such so the loop still works.

## Two ways to fill a gap

The new-game form chooses what happens when the model reaches a position you have never faced:

- **Stockfish fills in** (default): the engine plays at your rating and steers back toward your
  games, as described above.
- **Switch sides**: you take over the model's side and play the position yourself, as if you were
  the opponent, while the model plays your old side from your history. When the model runs out of
  history on that side too, you switch again. Because your moves are remembered, every switch
  fills exactly the gap that caused it, so the model learns where it was blank. The board flips,
  the move list marks each takeover, undo restores the side you had, and the PGN notes it.

## Games you play here are remembered

Every move you make in the app is added to your history at once, as a game with time class
`yvy` ("You v You" in the UI), so the model learns how you react to positions from your own
play, not only from chess.com. A take-back removes the memory again, the model's own moves are
never recorded, and the new-game form has a "Remember my moves" switch per game. Exclude app
games from the model with the time-class chips.

## Deploying

See [DEPLOY.md](DEPLOY.md): a $0 option behind Tailscale, a $5 VPS with the Compose stack in
`deploy/` (Caddy handles HTTPS), and Fly.io with scale-to-zero. Set `YVY_PASSWORD` first.

## Layout

```
crates/yvy-core     pure chess logic: position keys, PGN replay, sampling, game state (unit tests)
crates/yvy-server   axum API, chess.com client, importer, UCI driver, sessions, CLI, migrations
web                 Bun + React app (dev.ts is the dev server with the /api proxy)
docker-compose.yml  db service always; app service under the "full" profile
Dockerfile          bun build -> cargo build --release -> debian:trixie-slim + stockfish
deploy/             production Compose stack with Caddy, Fly config, env template
.github/workflows   CI: clippy + tests against Postgres, web typecheck/tests/build, image build
```

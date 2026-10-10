---
name: launch
description: Start the You v You Chess app locally (Postgres in Docker, the Rust API, the Bun web dev server) and report the URLs. Use this whenever the user wants to launch, start, run, boot, spin up, bring up or restart the app or its dev stack, wants to stop it, asks whether it is running, or wants to try a change in the browser. Use it instead of starting cargo, bun or docker compose by hand.
argument-hint: "[start|stop|status|logs|restart] [--open] [--db]"
---

# /launch

Everything lives in `scripts/launch.sh`, so starting the app is one deterministic command rather
than a sequence of fragile shell steps. The script loads `.env`, starts Docker Desktop when the
daemon is down, brings Postgres up with Compose and waits for its healthcheck, builds the Rust API
and starts it detached, starts the Bun dev server detached, waits until both answer, checks that
Stockfish was found, and prints the URLs. PIDs and logs go to `.run/`.

## Do this

1. From the repo root run `scripts/launch.sh $ARGUMENTS` in the foreground (no arguments means
   `start`). Allow up to five minutes the first time: a cold `cargo build` is the slow part.
2. Report the two URLs it prints (the UI on the Bun dev server and the API health endpoint) and
   whether Stockfish was found. Pass `--open` when the user wants the UI opened in their browser.
3. If it exits non-zero, the last line names the failing component and the fix. The usual ones:
   - Docker Desktop did not come up: ask the user to start it, then rerun.
   - A port is held by another process: the message says which port and which process. Change
     `YVY_BIND`, `PORT` or `YVY_DB_PORT` in `.env` (copy `.env.example` first if there is no
     `.env`) and rerun. On this machine 5432 and 8080 belong to other projects, which is why
     `.env` already points at 5433 and 8787.
   - `cargo build` failed: fix the compile error, then rerun.
   - Stockfish missing: the app still runs and fills gaps with random legal moves, clearly
     labelled; `brew install stockfish` fixes it.
   For anything else, `scripts/launch.sh logs` shows the last lines of both server logs.

## Why not start the servers by hand

`cargo run` and `bun run dev` block the shell, record no PIDs and leave orphans when killed, so
`stop`, `status` and a second `start` stop working. The script runs the built binary and
`bun --hot` directly and tracks them in `.run/`, which is what makes `scripts/launch.sh stop`
reliable. If something else already holds a port, `start` says so instead of guessing.

## Other actions

- `scripts/launch.sh status`: what is running, the health JSON, the engine line.
- `scripts/launch.sh stop`: stops the API and web servers. Add `--db` to stop Postgres as well.
- `scripts/launch.sh restart`: stop then start, for example after editing `.env`.
- `scripts/launch.sh logs [api|web]`: tail the logs.

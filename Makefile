# You v You Chess: developer commands. `make` alone runs the dev loop.
SHELL := /bin/bash
.PHONY: default db dev server web launch stop status test test-rust test-web build up down import lookup

default: dev

## Start Postgres in Docker and wait until it is healthy (needs Docker Desktop running).
db:
	@docker info >/dev/null 2>&1 || { echo "Docker daemon is not running. Start Docker Desktop first."; exit 1; }
	docker compose up -d db --wait

## Dev loop: Postgres in Docker, Rust server on :8080, Bun dev server with HMR on :3000.
dev: db
	@trap 'kill 0' EXIT; \
	(cd web && bun install --silent && bun run dev) & \
	cargo run -p yvy-server -- serve

## Same stack as `make dev`, but detached: pids and logs in .run/. Also available as the /launch skill.
launch:
	scripts/launch.sh start

stop:
	scripts/launch.sh stop

status:
	scripts/launch.sh status

## Only the Rust server (for use with `make web` in another terminal).
server: db
	cargo run -p yvy-server -- serve

## Only the Bun dev server.
web:
	cd web && bun install --silent && bun run dev

## All tests. Rust integration tests need the Compose Postgres.
test: db test-rust test-web

test-rust:
	@set -a; [ -f .env ] && . ./.env; set +a; \
	export DATABASE_URL="$${DATABASE_URL:-postgres://yvy:yvy@localhost:5432/yvy}"; \
	cargo test --workspace

test-web:
	cd web && bun install --silent && bun test

## Production build on the host: web bundle plus release binary.
build:
	cd web && bun install --frozen-lockfile && bun run build
	cargo build --release -p yvy-server

## Everything in Docker (Postgres + app image with Stockfish) on :8080.
up:
	docker compose --profile full up --build

down:
	docker compose --profile full down

## make import USER=someone [ARGS="--months 3"]
import: db
	cargo run -p yvy-server -- import $(USER) $(ARGS)

## make lookup USER=someone FEN="rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1"
lookup: db
	cargo run -p yvy-server -- lookup $(USER) "$(FEN)"

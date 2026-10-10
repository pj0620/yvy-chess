#!/usr/bin/env bash
# You v You Chess: start, stop or inspect the local dev stack.
#
#   scripts/launch.sh [start] [--open]   Postgres (Docker) + Rust API + Bun dev server, detached
#   scripts/launch.sh stop [--db]        stop the API and web servers (--db also stops Postgres)
#   scripts/launch.sh restart [--open]   stop, then start (use after editing .env)
#   scripts/launch.sh status             what is running and where
#   scripts/launch.sh logs [api|web]     last lines of the server logs
#
# PIDs and logs live in .run/ (gitignored). Settings come from .env, see .env.example.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
RUN_DIR="$ROOT/.run"
mkdir -p "$RUN_DIR"

# Same precedence as the servers themselves: a variable already in the environment wins over .env.
load_env() {
  [ -f .env ] || return 0
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in ''|'#'*) continue ;; esac
    key="${line%%=*}"
    val="${line#*=}"
    key="${key#"${key%%[![:space:]]*}"}"; key="${key%"${key##*[![:space:]]}"}"
    val="${val#\"}"; val="${val%\"}"; val="${val#\'}"; val="${val%\'}"
    [[ "$key" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]] || continue
    if [ -z "${!key+x}" ]; then export "$key=$val"; fi
  done < .env
}
load_env

BIND="${YVY_BIND:-127.0.0.1:8080}"
API_HOST="${BIND%:*}"; [ "$API_HOST" = "0.0.0.0" ] && API_HOST="127.0.0.1"
API_PORT="${BIND##*:}"
API_URL="http://${API_HOST}:${API_PORT}"
WEB_PORT="${PORT:-3000}"
WEB_URL="http://localhost:${WEB_PORT}"
DB_PORT="${YVY_DB_PORT:-5432}"
STOCKFISH="${STOCKFISH_PATH:-stockfish}"
API_BIN="$ROOT/target/debug/yvy-server"
WEB_ENTRY="$ROOT/web/dev.ts"

say()  { printf '%s\n' "$*"; }
ok()   { printf '  ok   %s\n' "$*"; }
warn() { printf '  warn %s\n' "$*"; }
die()  { printf '  FAIL %s\n' "$*" >&2; exit 1; }

docker_up()    { docker info >/dev/null 2>&1; }
healthy_api()  { curl -sf --max-time 2 "$API_URL/api/health" >/dev/null 2>&1; }
healthy_web()  { curl -sf --max-time 2 "$WEB_URL/" >/dev/null 2>&1; }
listener_pid() { lsof -nP -iTCP:"$1" -sTCP:LISTEN -t 2>/dev/null | head -n 1 || true; }

# wait_for <seconds> <predicate-function>
wait_for() {
  local secs=$1 fn=$2 i=0
  until "$fn"; do
    i=$((i + 1))
    [ "$i" -ge "$secs" ] && return 1
    sleep 1
  done
}

ensure_tools() {
  local t
  for t in docker cargo bun curl lsof; do
    command -v "$t" >/dev/null 2>&1 || die "$t is not installed (see README.md, Prerequisites)"
  done
  [ -f .env ] || warn "no .env found; using defaults (copy .env.example to .env to change ports)"
}

ensure_docker() {
  if docker_up; then ok "Docker daemon running"; return; fi
  if [ "$(uname -s)" = "Darwin" ] && [ -d /Applications/Docker.app ]; then
    say "  ...  starting Docker Desktop"
    open -a Docker
    wait_for 120 docker_up || die "Docker Desktop did not start within 2 minutes. Start it by hand, then rerun."
    ok "Docker daemon running"
  else
    die "the Docker daemon is not running. Start Docker, then rerun."
  fi
}

start_db() {
  if ! docker compose up -d db --wait >/dev/null 2>&1; then
    die "Postgres did not start. Try: docker compose up -d db --wait  (port $DB_PORT taken? set YVY_DB_PORT in .env)"
  fi
  ok "Postgres ready on localhost:$DB_PORT"
}

start_api() {
  if healthy_api; then ok "API already running at $API_URL"; return; fi
  local pid; pid=$(listener_pid "$API_PORT")
  if [ -n "$pid" ]; then
    die "port $API_PORT is held by pid $pid ($(ps -o comm= -p "$pid" 2>/dev/null)), which is not a healthy yvy API. Change YVY_BIND in .env or stop that process."
  fi
  say "  ...  building the API (the first build takes a few minutes)"
  cargo build -q -p yvy-server || die "cargo build failed; fix the errors above and rerun"
  nohup "$API_BIN" serve > "$RUN_DIR/api.log" 2>&1 &
  echo $! > "$RUN_DIR/api.pid"
  wait_for 60 healthy_api || die "the API did not become healthy within 60s; see .run/api.log"
  ok "API listening at $API_URL (pid $(cat "$RUN_DIR/api.pid"), log .run/api.log)"
}

start_web() {
  if healthy_web; then ok "web dev server already running at $WEB_URL"; return; fi
  local pid; pid=$(listener_pid "$WEB_PORT")
  if [ -n "$pid" ]; then
    die "port $WEB_PORT is held by pid $pid ($(ps -o comm= -p "$pid" 2>/dev/null)). Set PORT in .env to use another port, or stop that process."
  fi
  (cd web && bun install --silent) || die "bun install failed in web/"
  (
    cd web
    PORT="$WEB_PORT" nohup bun --hot "$WEB_ENTRY" > "$RUN_DIR/web.log" 2>&1 &
    echo $! > "$RUN_DIR/web.pid"
  )
  wait_for 30 healthy_web || die "the web dev server did not come up within 30s; see .run/web.log"
  ok "web dev server at $WEB_URL (pid $(cat "$RUN_DIR/web.pid"), log .run/web.log)"
}

check_engine() {
  local info; info=$(curl -sf --max-time 5 "$API_URL/api/engine" 2>/dev/null || true)
  if printf '%s' "$info" | grep -q '"available":true'; then
    ok "engine: $(printf '%s' "$info" | sed -E 's/.*"name":"([^"]*)".*/\1/')"
  else
    warn "Stockfish not found (STOCKFISH_PATH=$STOCKFISH). Gaps in your history get random legal moves until you install it: brew install stockfish"
  fi
}

open_url() {
  if command -v open >/dev/null 2>&1; then open "$1"
  elif command -v xdg-open >/dev/null 2>&1; then xdg-open "$1"
  fi
}

cmd_start() {
  local open_browser=0 a
  for a in "$@"; do
    case "$a" in
      --open) open_browser=1 ;;
      *) die "unknown option for start: $a" ;;
    esac
  done
  say "You v You Chess: starting the dev stack in $ROOT"
  ensure_tools
  ensure_docker
  start_db
  start_api
  start_web
  check_engine
  say ""
  say "  UI   $WEB_URL"
  say "  API  $API_URL/api/health"
  say "  stop: scripts/launch.sh stop    status: scripts/launch.sh status"
  if [ "$open_browser" = 1 ]; then open_url "$WEB_URL"; fi
}

stop_one() {
  local name=$1 f="$RUN_DIR/$1.pid" pid
  if [ -f "$f" ]; then
    pid=$(cat "$f")
    if kill -0 "$pid" 2>/dev/null; then
      kill "$pid" 2>/dev/null || true
      ok "$name stopped (pid $pid)"
    else
      say "  $name was not running"
    fi
    rm -f "$f"
  fi
}

cmd_stop() {
  local with_db=0 a
  for a in "$@"; do
    case "$a" in
      --db) with_db=1 ;;
      *) die "unknown option for stop: $a" ;;
    esac
  done
  stop_one api
  stop_one web
  # Servers started outside this script (or whose pid file was lost) for this checkout only.
  pkill -f "$API_BIN serve" 2>/dev/null && ok "stray API process stopped" || true
  pkill -f "bun --hot $WEB_ENTRY" 2>/dev/null && ok "stray web process stopped" || true
  if [ "$with_db" = 1 ]; then
    docker compose stop db >/dev/null 2>&1 && ok "Postgres stopped" || warn "could not stop Postgres"
  else
    say "  Postgres left running (scripts/launch.sh stop --db stops it too)"
  fi
}

cmd_status() {
  say "You v You Chess status ($ROOT)"
  if docker_up; then
    local st; st=$(docker compose ps --format '{{.Status}}' db 2>/dev/null || true)
    if [ -n "$st" ]; then ok "Postgres: $st (localhost:$DB_PORT)"; else warn "Postgres container is not running"; fi
  else
    warn "Docker daemon is not running"
  fi
  if healthy_api; then
    ok "API: $API_URL  $(curl -s --max-time 2 "$API_URL/api/health")"
    check_engine
  else
    warn "API not responding at $API_URL"
  fi
  if healthy_web; then ok "web: $WEB_URL"; else warn "web dev server not responding at $WEB_URL"; fi
}

cmd_logs() {
  local which="${1:-}" f
  case "$which" in
    api|web) tail -n 50 "$RUN_DIR/$which.log" 2>/dev/null || say "(no $which log yet)" ;;
    *)
      for f in api web; do
        say "== .run/$f.log =="
        tail -n 25 "$RUN_DIR/$f.log" 2>/dev/null || say "(no log yet)"
      done ;;
  esac
}

usage() { sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; }

cmd="${1:-start}"
if [ $# -gt 0 ]; then shift; fi
case "$cmd" in
  start|up)      cmd_start "$@" ;;
  stop|down)     cmd_stop "$@" ;;
  restart)       cmd_stop; cmd_start "$@" ;;
  status)        cmd_status ;;
  logs)          cmd_logs "$@" ;;
  -h|--help|help) usage ;;
  --*)           cmd_start "$cmd" "$@" ;;
  *)             usage; exit 2 ;;
esac

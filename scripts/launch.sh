#!/usr/bin/env bash
# You v You Chess: start, stop or inspect the local dev server.
#
#   scripts/launch.sh [start] [--open]   build the Rust backend to WebAssembly, start the Bun dev server
#   scripts/launch.sh stop               stop the dev server
#   scripts/launch.sh restart [--open]   stop, rebuild, start (use after changing Rust code)
#   scripts/launch.sh status             is it running, and where
#   scripts/launch.sh logs               last lines of the dev server log
#   scripts/launch.sh preview [--open]   production build served under /yvy-chess/, like GitHub Pages
#
# There is no API server or database: everything runs in the browser. PIDs and logs live in .run/.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
RUN_DIR="$ROOT/.run"
mkdir -p "$RUN_DIR"

if [ -f .env ]; then
  while IFS='=' read -r key val; do
    case "$key" in ''|'#'*) continue ;; esac
    [ -z "${!key+x}" ] && export "$key=$val"
  done < .env
fi

WEB_PORT="${PORT:-3000}"
WEB_URL="http://localhost:${WEB_PORT}"
PREVIEW_PORT="${PREVIEW_PORT:-4173}"
PREVIEW_URL="http://localhost:${PREVIEW_PORT}/yvy-chess/"

say()  { printf '%s\n' "$*"; }
ok()   { printf '  ok   %s\n' "$*"; }
warn() { printf '  warn %s\n' "$*"; }
die()  { printf '  FAIL %s\n' "$*" >&2; exit 1; }

healthy() { curl -sf --max-time 2 "$1" >/dev/null 2>&1; }
listener_pid() { lsof -nP -iTCP:"$1" -sTCP:LISTEN -t 2>/dev/null | head -n 1 || true; }
wait_for() { local secs=$1 url=$2 i=0; until healthy "$url"; do i=$((i + 1)); [ "$i" -ge "$secs" ] && return 1; sleep 1; done; }
open_url() { if command -v open >/dev/null 2>&1; then open "$1"; elif command -v xdg-open >/dev/null 2>&1; then xdg-open "$1"; fi; }

ensure_tools() {
  local t
  for t in cargo bun curl lsof; do
    command -v "$t" >/dev/null 2>&1 || die "$t is not installed (see README.md, Prerequisites)"
  done
  command -v wasm-bindgen >/dev/null 2>&1 || die "wasm-bindgen is missing: cargo install wasm-bindgen-cli --version 0.2.129 --locked"
  rustup target list --installed 2>/dev/null | grep -q wasm32-unknown-unknown || die "missing Rust target: rustup target add wasm32-unknown-unknown"
}

build_wasm() {
  say "  ...  building the Rust backend for WebAssembly"
  scripts/build-wasm.sh > "$RUN_DIR/wasm.log" 2>&1 || { tail -n 30 "$RUN_DIR/wasm.log"; die "the Rust build failed; see .run/wasm.log"; }
  ok "$(tail -n 1 "$RUN_DIR/wasm.log")"
}

start_web() {
  if healthy "$WEB_URL/yvy_bg.wasm"; then ok "dev server already running at $WEB_URL"; return; fi
  local pid; pid=$(listener_pid "$WEB_PORT")
  [ -n "$pid" ] && die "port $WEB_PORT is held by pid $pid ($(ps -o comm= -p "$pid" 2>/dev/null)). Set PORT in .env or stop it."
  (cd web && bun install --silent) || die "bun install failed in web/"
  (
    cd web
    PORT="$WEB_PORT" nohup bun --hot dev.ts > "$RUN_DIR/web.log" 2>&1 < /dev/null &
    echo $! > "$RUN_DIR/web.pid"
  )
  wait_for 30 "$WEB_URL/" || die "the dev server did not come up within 30s; see .run/web.log"
  ok "dev server at $WEB_URL (pid $(cat "$RUN_DIR/web.pid"), log .run/web.log)"
}

cmd_start() {
  local open_browser=0
  for a in "$@"; do case "$a" in --open) open_browser=1 ;; *) die "unknown option: $a" ;; esac; done
  say "You v You Chess: starting the dev server in $ROOT"
  ensure_tools
  build_wasm
  start_web
  say ""
  say "  UI   $WEB_URL"
  say "  stop: scripts/launch.sh stop    status: scripts/launch.sh status"
  [ "$open_browser" = 1 ] && open_url "$WEB_URL"
  return 0
}

stop_pidfile() {
  local name=$1 f="$RUN_DIR/$1.pid" pid
  [ -f "$f" ] || return 0
  pid=$(cat "$f")
  if kill -0 "$pid" 2>/dev/null; then kill "$pid" 2>/dev/null || true; ok "$name stopped (pid $pid)"; fi
  rm -f "$f"
}

cmd_stop() {
  stop_pidfile web
  stop_pidfile preview
  pkill -f "bun --hot dev.ts" 2>/dev/null && ok "stray dev server stopped" || true
  pkill -f "bun serve-dist.ts" 2>/dev/null && ok "stray preview server stopped" || true
  return 0
}

cmd_status() {
  say "You v You Chess status ($ROOT)"
  if healthy "$WEB_URL/yvy_bg.wasm"; then ok "dev server: $WEB_URL"; else warn "dev server not responding at $WEB_URL"; fi
  if healthy "$PREVIEW_URL"; then ok "preview: $PREVIEW_URL"; fi
  [ -f web/src/wasm/yvy_bg.wasm ] && ok "wasm built $(date -r web/src/wasm/yvy_bg.wasm '+%Y-%m-%d %H:%M')" || warn "wasm not built yet"
}

cmd_preview() {
  local open_browser=0
  for a in "$@"; do case "$a" in --open) open_browser=1 ;; *) die "unknown option: $a" ;; esac; done
  ensure_tools
  say "  ...  production build (wasm + static site)"
  (cd web && bun install --silent && bun run build) > "$RUN_DIR/build.log" 2>&1 || { tail -n 30 "$RUN_DIR/build.log"; die "build failed; see .run/build.log"; }
  ok "$(grep '^built web/dist' "$RUN_DIR/build.log" | tail -n 1)"
  stop_pidfile preview
  (
    cd web
    PORT="$PREVIEW_PORT" nohup bun serve-dist.ts > "$RUN_DIR/preview.log" 2>&1 < /dev/null &
    echo $! > "$RUN_DIR/preview.pid"
  )
  wait_for 15 "$PREVIEW_URL" || die "the preview server did not start; see .run/preview.log"
  ok "preview at $PREVIEW_URL (served under a subpath, like GitHub Pages)"
  [ "$open_browser" = 1 ] && open_url "$PREVIEW_URL"
  return 0
}

usage() { sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; }

cmd="${1:-start}"
[ $# -gt 0 ] && shift
case "$cmd" in
  start|up)      cmd_start "$@" ;;
  stop|down)     cmd_stop ;;
  restart)       cmd_stop; cmd_start "$@" ;;
  status)        cmd_status ;;
  logs)          tail -n 50 "$RUN_DIR/web.log" 2>/dev/null || say "(no log yet)" ;;
  preview)       cmd_preview "$@" ;;
  -h|--help|help) usage ;;
  --*)           cmd_start "$cmd" "$@" ;;
  *)             usage; exit 2 ;;
esac

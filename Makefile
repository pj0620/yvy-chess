# You v You Chess: developer commands. `make` alone runs the dev server in the foreground.
SHELL := /bin/bash
.PHONY: default dev wasm launch stop status test test-rust test-web build preview

default: dev

## Build the Rust backend to WebAssembly, then run the Bun dev server on :3000 (foreground).
dev: wasm
	cd web && bun install --silent && bun run dev

## Rust -> web/src/wasm (needs the wasm32 target and wasm-bindgen-cli 0.2.129).
wasm:
	scripts/build-wasm.sh

## Same as `make dev`, but detached, with pids and logs in .run/. Also the /launch skill.
launch:
	scripts/launch.sh start

stop:
	scripts/launch.sh stop

status:
	scripts/launch.sh status

## All tests: Rust unit and scenario tests, then the web tests against the real wasm module.
test: test-rust test-web

test-rust:
	cargo test --workspace

test-web: wasm
	cd web && bun install --silent && bun run typecheck && bun test

## The static site in web/dist, ready for GitHub Pages.
build:
	cd web && bun install --frozen-lockfile && bun run build

## Production build served under /yvy-chess/ on :4173, like GitHub Pages.
preview:
	scripts/launch.sh preview

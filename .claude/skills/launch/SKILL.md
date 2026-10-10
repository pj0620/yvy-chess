---
name: launch
description: Start the You v You Chess app locally (build the Rust backend to WebAssembly, run the Bun dev server) and report the URL, or serve the production build the way GitHub Pages will. Use this whenever the user wants to launch, start, run, boot, spin up, preview or restart the app, wants to stop it, asks whether it is running, or wants to try a change in the browser. Use it instead of running cargo, wasm-bindgen or bun by hand.
argument-hint: "[start|stop|status|logs|restart|preview] [--open]"
---

# /launch

The app is a static site: the backend is Rust compiled to WebAssembly and runs in the browser,
data lives in the browser's IndexedDB, and Stockfish runs in a Web Worker. There is no server
process or database. `scripts/launch.sh` builds the wasm module (`scripts/build-wasm.sh`), starts
the Bun dev server detached, waits until it answers, and prints the URL. PIDs and logs go to `.run/`.

## Do this

1. From the repo root run `scripts/launch.sh $ARGUMENTS` in the foreground (no arguments means
   `start`). The first Rust build takes a couple of minutes; later ones take seconds.
2. Report the URL it prints. Pass `--open` when the user wants it opened in their browser.
3. If it exits non-zero, the last line names the problem and the fix:
   - `wasm-bindgen is missing`: `cargo install wasm-bindgen-cli --version 0.2.129 --locked`
     (the version must match `crates/yvy-wasm/Cargo.toml`).
   - `missing Rust target`: `rustup target add wasm32-unknown-unknown`.
   - The Rust build failed: `.run/wasm.log` has the compiler output; fix it and rerun.
   - A port is taken: set `PORT` (or `PREVIEW_PORT`) in `.env` and rerun.

## Other actions

- `scripts/launch.sh restart`: stop, rebuild the wasm module, start. Use it after changing Rust
  code; TypeScript changes reload on their own.
- `scripts/launch.sh preview`: production build (`web/dist`) served under `/yvy-chess/` on :4173,
  exactly as GitHub Pages serves it. Use it to check a deploy before pushing.
- `scripts/launch.sh status`, `scripts/launch.sh stop`, `scripts/launch.sh logs`.

Each origin (localhost:3000, localhost:4173, the Pages site) has its own browser storage, so
imported games and games played in one are not visible in the others. Use the Save backup /
Restore backup buttons on a player page to move games played here between them.

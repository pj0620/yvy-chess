#!/usr/bin/env bash
# Builds the Rust backend for the browser: yvy-wasm -> web/src/wasm/{yvy.js, yvy.d.ts, yvy_bg.wasm}.
# Needs the wasm32 target and a wasm-bindgen CLI matching crates/yvy-wasm/Cargo.toml:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version 0.2.129 --locked
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
PROFILE="${1:-release}"
if [ "$PROFILE" = "release" ]; then FLAG="--release"; else FLAG=""; fi
cargo build -p yvy-wasm --target wasm32-unknown-unknown $FLAG
wasm-bindgen --target web --out-dir web/src/wasm --out-name yvy \
  "target/wasm32-unknown-unknown/$PROFILE/yvy_wasm.wasm"
printf 'built web/src/wasm/yvy_bg.wasm (%s KB)\n' "$(( $(wc -c < web/src/wasm/yvy_bg.wasm) / 1024 ))"

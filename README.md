# You v You Chess

Play chess against a model of yourself, built from your chess.com history.

Enter a chess.com username. The app downloads every game the account has played, replays them,
and indexes every decision the player made: the position they faced and the move they chose.
Then you play against that index. When the board reaches a position you have faced before, your
opponent answers with one of your own moves, picked in proportion to how often you played it.
When it reaches a position you have never faced, Stockfish fills the gap at a strength you
choose, or you switch sides and play it yourself. Every reply is labelled with where it came from.

It is a static site: everything runs in your browser. The logic is Rust compiled to
WebAssembly, your games are stored in the browser, and Stockfish runs in a Web Worker. There is
no server and nothing is uploaded.

Live: https://pj0620.github.io/yvy-chess/ (once GitHub Pages is enabled; see [DEPLOY.md](DEPLOY.md)).

## How the model works

- **Index.** Every position is keyed by a 64-bit hash of piece placement, side to move,
  castling rights and the en-passant square (only when a capture is possible), so transpositions
  merge. Each decision you made is one record: position, move, game. Filters by time class, date
  range or rated games apply at lookup time.
- **History moves.** From a known position the model samples one of your moves weighted by how
  often you played it, or always plays your most frequent move.
- **Gaps, option one: Stockfish fills in.** Stockfish plays at your rating and steers back toward
  your games. Moves that transpose into positions you know, or recreate a piece setup you
  often have, are preferred when they cost at most a chosen amount of evaluation. Steered moves
  get their own badge.
- **Gaps, option two: switch sides.** You take over the model's side and play the position
  yourself, while the model plays your old side from your history.
- **Memory.** Your moves in games played here join your history (shown as "You v You" games), so
  the model fills its blanks as you play. Undo takes a memory back. Each game has a switch to
  turn this off.

## Your data

Imported games and games played here live in this browser's IndexedDB. Clearing site data
erases them. Imported games can always be fetched again from chess.com; games played here exist
only in the browser, so the player page has **Save backup** and **Restore backup** buttons.

## Develop

Prerequisites: Rust stable (`rust-toolchain.toml` adds the `wasm32-unknown-unknown` target),
[Bun](https://bun.sh) 1.2+, and the wasm-bindgen CLI at the version pinned in
`crates/yvy-wasm/Cargo.toml`:

```sh
cargo install wasm-bindgen-cli --version 0.2.129 --locked
```

| Command | What it does |
|---|---|
| `make dev` | Build the Rust backend to WebAssembly and run the dev server on http://localhost:3000 |
| `make launch` / `make stop` / `make status` | The same dev server, detached (`scripts/launch.sh`, also the `/launch` skill in Claude Code) |
| `make test` | Rust tests, then web typecheck and tests, including a test that drives the real `.wasm` |
| `make build` | The static site in `web/dist` |
| `make preview` | The static site served under `/yvy-chess/` on :4173, the way GitHub Pages serves it |

After changing Rust code, rebuild the wasm module (`make wasm` or `scripts/launch.sh restart`).
TypeScript changes reload on their own.

## Layout

```
crates/yvy-core    chess primitives: position hashes, PGN replay, move sampling, game rules
crates/yvy-app     the app's logic: index, chess.com import, sessions, steering, memory, PGN export
crates/yvy-wasm    wasm-bindgen wrapper; scripts/build-wasm.sh writes web/src/wasm
web/src/backend    browser side of the backend: IndexedDB, chess.com client, Stockfish worker
web/src            React UI (pages, components)
web/public/engine  Stockfish 19 lite single-threaded WebAssembly build (GPLv3, see its README)
.github/workflows  ci.yml (tests on every push), pages.yml (deploy to GitHub Pages from main)
```

The Rust side makes every decision. When the model needs the engine, it returns a request with
the position, strength and steering candidates; the page runs Stockfish and hands the result
back, and Rust picks the move. The scenarios that once ran against a Postgres-backed server now
run as native Rust tests in `crates/yvy-app/tests`.

## Credits

Engine: [Stockfish](https://github.com/official-stockfish/Stockfish) via
[stockfish.js](https://github.com/nmrugg/stockfish.js), GPLv3, shipped unmodified in
`web/public/engine` with its license. Chess logic: [shakmaty](https://github.com/niklasf/shakmaty)
and [pgn-reader](https://github.com/niklasf/rust-pgn-reader). Game data: the
[chess.com public API](https://www.chess.com/news/view/published-data-api).

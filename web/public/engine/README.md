# Stockfish 19 (lite, single-threaded) for the browser

`stockfish-19-lite-single.js` and `stockfish-19-lite-single.wasm` are unmodified files from the
[stockfish.js](https://github.com/nmrugg/stockfish.js) npm package, version 19.0.1, by Nathan Rugg,
a WebAssembly build of [Stockfish](https://github.com/official-stockfish/Stockfish).

Stockfish and stockfish.js are licensed under the GNU General Public License v3; see `COPYING.txt`.
Source code: https://github.com/nmrugg/stockfish.js/tree/v19.0.1 and
https://github.com/official-stockfish/Stockfish.

The app talks to the engine only through the UCI protocol in a Web Worker, as a separate program.
The single-threaded build needs no cross-origin isolation headers, so it runs on GitHub Pages.

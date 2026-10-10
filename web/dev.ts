// Dev server: the app with hot reload, plus the runtime assets the production build copies
// next to index.html (the Rust backend's .wasm and the Stockfish worker).
import index from "./src/index.html";

const port = Number(process.env.PORT ?? 3000);
const file = (path: string) => () => new Response(Bun.file(new URL(path, import.meta.url)));

const server = Bun.serve({
  port,
  development: { hmr: true, console: true },
  routes: {
    "/yvy_bg.wasm": file("./src/wasm/yvy_bg.wasm"),
    "/engine/stockfish-19-lite-single.js": file("./public/engine/stockfish-19-lite-single.js"),
    "/engine/stockfish-19-lite-single.wasm": file("./public/engine/stockfish-19-lite-single.wasm"),
    "/*": index,
  },
});

console.log(`You v You web: http://localhost:${server.port}`);

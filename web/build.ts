// Production build: a fully static site in web/dist, ready for GitHub Pages or any static host.
// Run scripts/build-wasm.sh first (package.json "build" does both).
import { cp, mkdir, rm, stat, writeFile } from "node:fs/promises";
import { join } from "node:path";

const root = import.meta.dir;
const out = join(root, "dist");
const wasm = join(root, "src/wasm/yvy_bg.wasm");

try {
  await stat(wasm);
} catch {
  console.error("src/wasm/yvy_bg.wasm is missing: run ../scripts/build-wasm.sh first");
  process.exit(1);
}

await rm(out, { recursive: true, force: true });
const result = await Bun.build({
  entrypoints: [join(root, "src/index.html")],
  outdir: out,
  minify: true,
  target: "browser",
  sourcemap: "linked",
});
if (!result.success) {
  for (const log of result.logs) console.error(log);
  process.exit(1);
}

// Assets loaded at runtime by URL, relative to the page: the Rust backend and the engine.
await cp(wasm, join(out, "yvy_bg.wasm"));
await mkdir(join(out, "engine"), { recursive: true });
await cp(join(root, "public/engine"), join(out, "engine"), { recursive: true });
// GitHub Pages: serve files as they are, without Jekyll processing.
await writeFile(join(out, ".nojekyll"), "");

const files = [...result.outputs.map((o) => o.path), join(out, "yvy_bg.wasm"), join(out, "engine/stockfish-19-lite-single.wasm")];
let total = 0;
for (const f of files) total += (await stat(f)).size;
console.log(`built web/dist (${(total / 1024 / 1024).toFixed(1)} MB including the engine and the Rust backend)`);

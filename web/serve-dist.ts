// Serves web/dist under a subpath, the way GitHub Pages serves a project site
// (https://<user>.github.io/<repo>/), to test the production build locally:
//   bun serve-dist.ts            -> http://localhost:4173/yvy-chess/
import { join, normalize } from "node:path";

const base = `/${(process.env.BASE ?? "yvy-chess").replace(/^\/|\/$/g, "")}/`;
const port = Number(process.env.PORT ?? 4173);
const dist = join(import.meta.dir, "dist");

const server = Bun.serve({
  port,
  async fetch(req) {
    const { pathname } = new URL(req.url);
    if (pathname === base.slice(0, -1)) return Response.redirect(base, 301);
    if (!pathname.startsWith(base)) return new Response("Not found", { status: 404 });
    const rel = normalize(decodeURIComponent(pathname.slice(base.length)) || "index.html");
    if (rel.startsWith("..")) return new Response("Bad path", { status: 400 });
    const f = Bun.file(join(dist, rel.endsWith("/") ? `${rel}index.html` : rel));
    return (await f.exists()) ? new Response(f) : new Response("Not found", { status: 404 });
  },
});
console.log(`serving web/dist at http://localhost:${server.port}${base}`);

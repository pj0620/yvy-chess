// Bun dev server: serves the React app with HMR and proxies /api/* to the Rust server.
import index from "./src/index.html";

// The repo-level .env (if any) decides where the Rust server listens, so read it here too.
async function loadRepoEnv(): Promise<Record<string, string>> {
  const out: Record<string, string> = {};
  try {
    const text = await Bun.file(`${import.meta.dir}/../.env`).text();
    for (const line of text.split("\n")) {
      const m = line.match(/^\s*([A-Z0-9_]+)\s*=\s*(.*?)\s*$/);
      if (m) out[m[1]] = m[2].replace(/^["']|["']$/g, "");
    }
  } catch {
    // no .env, defaults apply
  }
  return out;
}

const repoEnv = await loadRepoEnv();
const bind = (process.env.YVY_BIND ?? repoEnv.YVY_BIND ?? "127.0.0.1:8080").replace(/^0\.0\.0\.0/, "127.0.0.1");
const API = process.env.YVY_API ?? `http://${bind}`;
const port = Number(process.env.PORT ?? 3000);

async function proxy(req: Request): Promise<Response> {
  const url = new URL(req.url);
  const target = `${API}${url.pathname}${url.search}`;
  const headers = new Headers(req.headers);
  headers.delete("host");
  const init: RequestInit = { method: req.method, headers, redirect: "manual" };
  if (req.method !== "GET" && req.method !== "HEAD") {
    init.body = await req.arrayBuffer();
  }
  try {
    const res = await fetch(target, init);
    const out = new Headers(res.headers);
    out.delete("content-length");
    return new Response(res.body, { status: res.status, headers: out });
  } catch (e) {
    return Response.json(
      { error: `The Rust server is not reachable at ${API}: ${(e as Error).message}` },
      { status: 502 },
    );
  }
}

const server = Bun.serve({
  port,
  development: { hmr: true, console: true },
  routes: {
    "/api/*": proxy,
    "/*": index,
  },
});

console.log(`You v You web: http://localhost:${server.port}  (API proxied to ${API})`);

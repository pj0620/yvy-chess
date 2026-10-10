# Deploying You v You Chess

The app is a static site, so hosting is free. `make build` (or `cd web && bun run build`)
writes everything to `web/dist`, about 3.5 MB in all:

- the page and its JavaScript;
- `yvy_bg.wasm`, the Rust backend, about 1.3 MB;
- `engine/`, Stockfish's single-threaded WebAssembly build, about 1.8 MB.

All paths are relative and routes use the URL hash (`#/p/name`), so the site works from any
subpath and needs no server rewrites. The single-threaded engine needs no special headers.

## GitHub Pages (recommended)

The repo has a workflow, `.github/workflows/pages.yml`, that builds and publishes on every push to `main`.

1. Enable Pages with GitHub Actions as the source, once. Either use the repository settings:
   **Settings → Pages → Build and deployment → Source: GitHub Actions**. Or the CLI:

   ```sh
   gh api -X POST repos/pj0620/yvy-chess/pages -f build_type=workflow
   ```

2. Push to `main`, or run the workflow by hand from the Actions tab. The site appears at
   `https://pj0620.github.io/yvy-chess/`.

To check a build locally exactly as Pages will serve it, run `make preview` and open
http://localhost:4173/yvy-chess/.

A custom domain works too: add it under Settings → Pages and point a CNAME record at
`pj0620.github.io`.

## Other static hosts

Any static host works with the same `web/dist` folder: Cloudflare Pages, Netlify, Vercel, an S3
bucket, or `python3 -m http.server` inside `web/dist`. The only requirement is that `.wasm` files
are served as `application/wasm`, which every mainstream host already does.

## Things to know

- **Data is per browser and per site.** Each visitor's imports and games live in their own
  browser's IndexedDB, so there is nothing shared to protect and no accounts are needed.
  Moving to another device or domain means using Save backup and Restore backup on the player
  page.
- **chess.com rate limits.** Imports fetch one month at a time and back off when chess.com
  throttles. A very large account can take a few minutes the first time; later re-syncs only
  fetch the current month.
- **Stockfish licensing.** The engine files are GPLv3 and ship unmodified with their license and
  source links in `engine/README.md` and `engine/COPYING.txt`.

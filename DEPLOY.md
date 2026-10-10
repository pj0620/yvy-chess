# Deploying You v You Chess

What runs: one container with the Rust server, the web bundle and Stockfish (about 200 MB), plus
Postgres. A big account's history is a few hundred MB in Postgres. Stockfish thinks for
300 ms per gap move, so any single shared vCPU is fine, and there is no GPU or other service.
Plan on 1 GB of RAM for app plus database; 512 MB works with `YVY_ENGINE_MOVETIME_MS` lowered,
but it is tight.

**Before exposing it to the internet, set `YVY_PASSWORD`.** Every request except the health
check then needs that password over HTTP Basic auth (the browser asks once). Without it anyone
who finds the URL can trigger chess.com imports and play games that get written into your
history. Behind Tailscale or on a home network the password is optional.

## Options, cheapest first

Prices checked in October 2026; they drift, so treat them as rough.

| Option | Cost per month | Good for | Watch out for |
|---|---|---|---|
| Your own Mac plus Tailscale | $0 | Just you, from your phone or laptop anywhere | Only works while the Mac is awake |
| Oracle Cloud Always Free ARM VM | $0 | A real always-on server for free | Free allocation cut to 2 OCPU / 12 GB in June 2026; sign-up friction; idle VMs can be reclaimed |
| Hetzner CX23 (2 vCPU, 4 GB) | about €4.50 (€3.99 plus €0.50 IPv4) | Best price for a real server | Cost-optimized plans were listed as unavailable in September 2026; CAX11 ARM at €4.49 is the fallback |
| DigitalOcean Basic Droplet | $4 (512 MB) or $6 (1 GB) | Same idea, dollar billing | 512 MB is too small for app plus Postgres; take the $6 one |
| Fly.io, machine plus Fly Postgres | about $2 to $8 | Scale to zero, no server to maintain | Idle machine stops, so the first request waits a few seconds; Fly Postgres is self-managed |
| Railway | $5 floor plus usage, realistically $10 to $15 | Simplest dashboard | Metered RAM and CPU add up for an always-on Postgres |

Recommendation: if it is only for you, run it at home behind Tailscale for nothing. If you want
it always on, a Hetzner or DigitalOcean box with the Compose file below is predictable at about
$5 a month and runs Stockfish well. Pick Fly if you would rather not own a server and can live
with a cold start.

## A. Your Mac plus Tailscale ($0)

1. Install Tailscale on the Mac and on your phone or laptop.
2. Start the full stack: `make up` (or `scripts/launch.sh` for the dev stack).
3. `tailscale serve --bg 8080` gives you an HTTPS URL on your tailnet that proxies to the app.
   Use `YVY_APP_PORT` from `.env` if you changed the port.

No password needed; only devices on your tailnet can reach it.

## B. A small VPS with Docker Compose (about $5)

Works the same on Hetzner, DigitalOcean, Oracle's free ARM VM or anything with Docker.

```sh
# on the server, as root, Ubuntu or Debian
curl -fsSL https://get.docker.com | sh
git clone <your repo url> yvy_chess && cd yvy_chess
cp deploy/.env.prod.example deploy/.env.prod
nano deploy/.env.prod            # DOMAIN, POSTGRES_PASSWORD, YVY_CONTACT, YVY_PASSWORD
```

Point your domain's A record (and AAAA if you have IPv6) at the server, then:

```sh
docker compose -f deploy/docker-compose.prod.yml --env-file deploy/.env.prod up -d --build
```

Caddy obtains the certificate on first start, so open ports 80 and 443 in the provider's
firewall. The first build compiles the Rust server in release mode, which takes several minutes
on a 2 vCPU box; later builds reuse the cache. If the server is too small to build comfortably,
build the image elsewhere and push it to a registry (GitHub Container Registry is free for
public images), then replace the `build:` block in the Compose file with `image: ...`. On Apple
Silicon build for the server's architecture: `docker build --platform linux/amd64 -t ghcr.io/you/yvy-chess .`.

Day to day:

- Update: `git pull && docker compose -f deploy/docker-compose.prod.yml --env-file deploy/.env.prod up -d --build`
- Logs: `docker compose -f deploy/docker-compose.prod.yml logs -f app`
- Backup (weekly cron is plenty; the chess.com games can always be re-imported, your own games played here are the part worth keeping):
  `docker compose -f deploy/docker-compose.prod.yml exec -T db pg_dump -U yvy yvy | gzip > yvy-$(date +%F).sql.gz`
- Restore: `gunzip -c yvy-DATE.sql.gz | docker compose -f deploy/docker-compose.prod.yml exec -T db psql -U yvy yvy`

## C. Fly.io (about $2 to $8)

```sh
fly launch --no-deploy --copy-config --config deploy/fly.toml   # pick an app name and region
fly postgres create                                              # smallest development size
fly postgres attach <pg-app> -a <app>                            # sets DATABASE_URL
fly secrets set YVY_PASSWORD=... YVY_CONTACT=you@example.com -a <app>
fly deploy --config deploy/fly.toml --dockerfile Dockerfile
```

`deploy/fly.toml` stops the machine when idle and starts it on the next request, so you pay only
for the hours it is awake plus the Postgres machine and volume. The health check path is
`/api/health`, which the password gate leaves open on purpose.

## What the app needs from the platform

- `DATABASE_URL`, `YVY_BIND=0.0.0.0:8080`, `YVY_WEB_DIR=/srv/web`, `STOCKFISH_PATH=/usr/games/stockfish`
  (the image sets the last three), `YVY_CONTACT`, and ideally `YVY_PASSWORD`.
- Migrations run at startup, so a fresh database is fine.
- `yvy-server health` inside the container exits 0 when the server answers; the Dockerfile and
  the Compose files use it as the health check.
- One CPU is enough. Two concurrent games share the single Stockfish process and simply wait
  for each other.

## Sources for the prices

- [Hetzner cost-optimized plans](https://www.bitdoze.com/hetzner-cloud-cost-optimized-plans), [Hetzner 2026 price increase](https://findstack.com/resources/hetzner-price-increase-2026), [Hetzner availability notes](https://www.vincentschmalbach.com/hetzner-cheap-cloud-unavailable-price-increases/)
- [Fly.io pricing](https://fly.io/pricing.md), [Fly.io real costs 2026](https://servercompass.app/blog/flyio-pricing-breakdown-vs-vps)
- [Oracle free tier changes 2026](https://terminalbytes.com/oracle-cloud-free-tier-changes-2026), [Oracle free tier breakdown](https://fullmetalbrackets.com/blog/oci-free-tier-breakdown)
- [Railway pricing explained](https://livemy.app/blog/railway-pricing), [DigitalOcean pricing 2026](https://diyai.io/ai-tools/hosting/digitalocean-pricing/)

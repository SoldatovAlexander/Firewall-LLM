# Deployment

**Docker Compose (on-prem, auto-start `unless-stopped`):**
```bash
cd deploy
cp fwllm.yaml.example fwllm.yaml
cp .env.example .env
docker compose up -d --build
```

**Helm — out of release 0.1.0 scope** (possible later): `helm install fwllm ./deploy/helm/fwllm --set secret.openRouterApiKey=... --set secret.clientTokens="..."`. The chart is only statically verified (lint/template/kubeconform) and was never installed on a live cluster.

## Python gateway capacity (0.1.1)

- One worker holds ~5–20 chat rps on weak hardware (measured); the image runs `UVICORN_WORKERS=4` by default — scales with processes.
- Metrics under workers aggregate via `PROMETHEUS_MULTIPROC_DIR` (container-local, baked into the image).
- The routing budget mirror is a per-worker approximation; Redis quotas stay exact.
- Audit: WAL + `busy_timeout`, no fsync per commit (cheap and concurrent; money lives in Redis).
- Overload is cut with an honest 429 (`server.max_inflight_requests`, default 32 per worker), not queued until timeouts.

## What survives a restart (release 0.1.0)

- **Quotas and reservations** — in Redis (daily buckets + `fwllm:rsv:*`); a gateway restart does not reset them.
- **Audit** — SQLite on a volume (`audit.db_path`, `./data` in Compose); new columns (`usage_source`, `request_id`) migrate automatically.
- **Does not survive**: Rust gateway routing-rule budgets (in-memory mirror; metering is the source of truth), in-flight streams (clients reconnect; aborted ones are accounted as `cancelled`).
- Unsettled reservations after a crash are never refunded (conservative R05 policy): the budget stays consumed until day rollover.

**Bench:** `docker compose -f docker-compose.yml -f docker-compose.bench.yml up -d`

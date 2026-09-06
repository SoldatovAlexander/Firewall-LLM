# Deployment

**Docker Compose (on-prem, auto-start `unless-stopped`):**
```bash
cd deploy
cp fwllm.yaml.example fwllm.yaml
cp .env.example .env
docker compose up -d --build
```

**Helm — out of release 0.1.0 scope** (possible later): `helm install fwllm ./deploy/helm/fwllm --set secret.openRouterApiKey=... --set secret.clientTokens="..."`. The chart is only statically verified (lint/template/kubeconform) and was never installed on a live cluster.

## What survives a restart (release 0.1.0)

- **Quotas and reservations** — in Redis (daily buckets + `fwllm:rsv:*`); a gateway restart does not reset them.
- **Audit** — SQLite on a volume (`audit.db_path`, `./data` in Compose); new columns (`usage_source`, `request_id`) migrate automatically.
- **Does not survive**: Rust gateway routing-rule budgets (in-memory mirror; metering is the source of truth), in-flight streams (clients reconnect; aborted ones are accounted as `cancelled`).
- Unsettled reservations after a crash are never refunded (conservative R05 policy): the budget stays consumed until day rollover.

**Bench:** `docker compose -f docker-compose.yml -f docker-compose.bench.yml up -d`

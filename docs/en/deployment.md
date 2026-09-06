# Deployment

**Docker Compose (on-prem, auto-start `unless-stopped`):**
```bash
cd deploy
cp fwllm.yaml.example fwllm.yaml
cp .env.example .env
docker compose up -d --build
```

**Helm — out of release 0.1.0 scope** (possible later): `helm install fwllm ./deploy/helm/fwllm --set secret.openRouterApiKey=... --set secret.clientTokens="..."`. The chart is only statically verified (lint/template/kubeconform) and was never installed on a live cluster.

**Bench:** `docker compose -f docker-compose.yml -f docker-compose.bench.yml up -d`

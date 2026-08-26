# Deployment

**Docker Compose (on-prem, auto-start `unless-stopped`):**
```bash
cd deploy
cp fwllm.yaml.example fwllm.yaml
cp .env.example .env
docker compose up -d --build
```

**Helm:** `helm install fwllm ./deploy/helm/fwllm --set secret.openRouterApiKey=...`

**Bench:** `docker compose -f docker-compose.yml -f docker-compose.bench.yml up -d`

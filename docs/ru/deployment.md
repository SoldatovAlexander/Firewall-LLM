# Развёртывание

**Docker Compose:**
```bash
cd deploy
cp fwllm.yaml.example fwllm.yaml
cp .env.example .env
docker compose up -d --build
```

**Helm — вне скоупа релиза 0.1.0** (возможен позже): `helm install fwllm ./deploy/helm/fwllm --set secret.openRouterApiKey=... --set secret.clientTokens="..."`. Чарт проверен только статически (lint/template/kubeconform), на живой кластер не ставился.

**Бенч:** `docker compose -f docker-compose.yml -f docker-compose.bench.yml up -d`

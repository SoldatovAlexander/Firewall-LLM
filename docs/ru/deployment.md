# Развёртывание

**Docker Compose:**
```bash
cd deploy
cp fwllm.yaml.example fwllm.yaml
cp .env.example .env
docker compose up -d --build
```

**Helm — вне скоупа релиза 0.1.0** (возможен позже): `helm install fwllm ./deploy/helm/fwllm --set secret.openRouterApiKey=... --set secret.clientTokens="..."`. Чарт проверен только статически (lint/template/kubeconform), на живой кластер не ставился.

## Что переживает рестарт (релиз 0.1.0)

- **Квоты и резервы** — в Redis (daily buckets + `fwllm:rsv:*`); рестарт гейта их не сбрасывает.
- **Аудит** — SQLite на volume (`audit.db_path`, в Compose `./data`); новые колонки (`usage_source`, `request_id`) мигрируют автоматически.
- **Не переживает**: бюджеты routing-правил Rust-гейта (in-memory mirror, источник правды — metering), in-flight стримы (клиент переподключается; учёт оборванных — `cancelled`).
- Незавершённые резервы после падения не возвращаются (консервативная политика R05): бюджет остаётся занятым до конца суток.

**Бенч:** `docker compose -f docker-compose.yml -f docker-compose.bench.yml up -d`

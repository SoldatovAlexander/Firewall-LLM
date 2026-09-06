# Развёртывание

**Docker Compose:**
```bash
cd deploy
cp fwllm.yaml.example fwllm.yaml
cp .env.example .env
docker compose up -d --build
```

**Helm — вне скоупа релиза 0.1.0** (возможен позже): `helm install fwllm ./deploy/helm/fwllm --set secret.openRouterApiKey=... --set secret.clientTokens="..."`. Чарт проверен только статически (lint/template/kubeconform), на живой кластер не ставился.

## Пропускная способность Python-гейта (0.1.1)

- Один воркер держит ~5–20 rps chat на слабом CPU (замерено); образ запускает `UVICORN_WORKERS=4` по умолчанию — масштабируется числом процессов.
- Метрики при воркерах агрегируются через `PROMETHEUS_MULTIPROC_DIR` (container-local, в образе).
- Routing-зеркало бюджетов — per-worker аппроксимация; точными остаются квоты в Redis.
- Аудит: WAL + `busy_timeout`, без fsync на коммит (дешево и многопоточно; деньги — в Redis).
- Перегрузка режется честным 429 (`server.max_inflight_requests`, по умолчанию 32 на воркер), а не очередью до таймаутов.

## Бэкап аудита

`scripts/backup-audit.sh <data-dir> <backup-dir> [keep-days=7]` — консистентная копия через SQLite backup API (без гонок живого файла), проверка `integrity_check`, gzip, ротация. Для cron/systemd: раз в сутки ночью.

## Что переживает рестарт (релиз 0.1.0)

- **Квоты и резервы** — в Redis (daily buckets + `fwllm:rsv:*`); рестарт гейта их не сбрасывает.
- **Аудит** — SQLite на volume (`audit.db_path`, в Compose `./data`); новые колонки (`usage_source`, `request_id`) мигрируют автоматически.
- **Не переживает**: бюджеты routing-правил Rust-гейта (in-memory mirror, источник правды — metering), in-flight стримы (клиент переподключается; учёт оборванных — `cancelled`).
- Незавершённые резервы после падения не возвращаются (консервативная политика R05): бюджет остаётся занятым до конца суток.

**Бенч:** `docker compose -f docker-compose.yml -f docker-compose.bench.yml up -d`

# Итоговый отчёт по тестам Firewall LLM

**Дата:** 2026-09-06 (обновлён после цикла релизных исправлений R01–R16) · **Парадигма:** TDD (Red → Green → Refactor) · **Всего:** 263 теста (182 Python + 81 Rust), 1 live (пропущен без ключа)

Исходная версия от 2026-08-26 — 181 тест (144 + 37). Дельта +82 — цикл R01–R16, см. §5.

---

## 1. Сводная таблица

| № | Модуль / Этап | Файлы тестов | Тестов | Статус | Пояснение |
|---|---------------|--------------|--------|--------|-----------|
| 1 | **Фундамент** — конфиг-загрузчик | `test_config.py` | 8 | ✅ | YAML + `FWLLM_*` env-переопределения, резолв `api_key` из env, валидация обязательных полей, TTL vault. Ошибки — `ConfigError` с полем `base_url` |
| 2 | **Контракты** — OpenAPI + JSON Schema | `test_contracts.py` | 3 | ✅ | Парсинг `openapi.yaml`, валидация `policies.schema.json` (draft2020), пример `policy.example.yaml` соответствует схеме |
| 3 | **Gateway** — единый вход | `test_gateway.py` | 18 | ✅ | `/healthz`, `/metrics`, Bearer 401, 422 на невалидном теле, passthrough ответа, 502 на upstream, 403 на `BlockedError`, SSE `data: [DONE]` и in-stream error, контрактные диапазоны (R07) |
| 4 | **Адаптеры** — провайдеры | `test_adapters_contract.py`, `test_registry.py`, `test_live_openrouter.py` | 18 + 4 + 1 live | ✅ | Контрактный набор для 3 адаптеров (respx): Bearer, статусы → `ProviderError`, сеть → `ProviderError`, стрим до `[DONE]`; реестр `type` → класс |
| 5 | **Metering** — учёт и квоты | `test_metering.py` (9), `test_gateway_metering.py` (10), `test_metering_atomic.py` (4, R05) | 23 | ✅ | Redis daily buckets, атомарный admit/reserve + идемпотентный settle (Lua + fallback), 20 конкурентных admits → 1 слот, always-count стриминга (R03), estimate + `usage_source`, `include_usage`, genuine-disconnect через raw ASGI |
| 6 | **Observability** — метрики | `test_metrics.py` (2), `test_gateway_metrics.py` (5) | 7 | ✅ | `fw_requests_total{client,provider,model,code}`, `fw_tokens_total{direction}`, `fw_request_duration_seconds`, `fw_audit_errors_total` (R12); metrics-токены R15: scrape без admin 200, чужой 401, клиентский 403 |
| 7 | **Inspector** — DLP + injection | `test_inspectors.py` (16), `test_gateway_inspectors.py` (4) | 20 | ✅ | LightAnon RAG: `sanitize/restore`, vault TTL, политика `mask/restore`; сигнатуры injection с severity, корд-тест precision/recall; DLP не пропускает PII наружу; stateful stream-restore поперёк чанков (R13), скан `tool_calls/name` (R07) |
| 8 | **Egress** — прокси | `test_egress.py` | 7 | ✅ | MVP два режима: `direct` и `single_proxy` (один прокси на все адаптеры), `trust_env=False`; валидация `proxy_url` обязателен |
| 9 | **Router + Policy** — переключение | `test_router.py` (10), `test_gateway_router.py` (4), `test_router_persistence.py` (5), `test_gateway_router_store.py` (1), `test_router_failopen.py` (3) | 23 | ✅ | Chain resolution, `model_mapping`, бюджетные правила `provider_tokens_today > N` → `next_in_chain`, attack failover `N=5/window=300s` → `block_source` + `switch_to` с cooldown, персистентность в Redis |
| 10 | **Audit** — лог | `test_audit.py` (5), `test_gateway_audit.py` (7) | 12 | ✅ | SQLite append-only, редакция PII до записи, `search` по `client/code` с `limit` newest-first, `/admin/audit` 401/200, `enabled:false` не пишет; `request_id`/`usage_source` + миграции (R03/R12), truncate 8000, disconnect → `cancelled` ровно одна строка |
| 11 | **Enterprise: пулы прокси** | `test_egress_pools.py`, `test_adapter_pools.py` | 7 + 4 | ✅ | `ProxyPool` round_robin/random/least_used, `requests_per_proxy` ротация, `fail_threshold`+cooldown, `bindings` валидация |
| 12 | **Enterprise: ML-детектор** | `test_ml_inspector.py` | 11 | ✅ | Fake ONNX session + tokenizer, `OnnxTextClassifier` softmax, пороги severity (0.9/0.8/0.7), `MlInjectionInspector` block/log, graceful `None` без `onnxruntime`, цепочка из 3 инспекторов |
| 13 | **Rust: core + gateway** (без туннеля — см. №14) | `fwllm-core/tests/config_test.rs` (11), `fwllm-agent` unit (3), `fwllm-gateway/tests/`: audit (5), egress (6), gateway (25), ingress (7), inspectors (3), lifecycle (5, R12), metering (9) | 74 | ✅ | Порт конфига (env + api_key_env + metrics_tokens), gateway: healthz/metrics, 401/422/502, `routed_from`, SSE, атомарный admit/settle + конкурентность (R05), audit lifecycle + `request_id` (R12), DLP/injection + stream-restore (R13), routing-бюджеты (R06), ingress WS 101/401, `state_store=redis` запрещён (R06) |
| 14 | **Ingress tunnel** | `e2e_tunnel_test.rs` (1), `tunnel_test.rs` (2), `real_tunnel_test.rs` (4, R09) | 7 | ✅ | `mask_for_tunnel` режет `Via/X-Forwarded-*`, `TunnelProvider` через `mpsc` канал, E2E не видит исходные заголовки; настоящие бинарники gateway+agent: WSS-handshake, completion через туннель, reject wrong token/CA, `--insecure` |
| 15 | **Прочее** | `test_admin_auth.py` (6), `test_main.py` (3), `test_grafana_import.py` (4), `test_stream_security.py` (4) | 17 | ✅ | Admin/self-audit матрица (R01), входные точки, импорт дашборда, безопасность стрима |

**Итого Python:** 182 (+1 live `test_live_openrouter.py`, пропускается без ключа). **Итого Rust:** 81 (74 №13 + 7 №14). **Всего: 263.**

---

## 5. Цикл релизных исправлений R01–R16 (2026-09-06, TDD Red→Green)

По `docs/RELEASE_CODE_REVIEW_2026-09-06.md` и `docs/PLAN_FIX_RELEASE_2026-09-06.md`. Каждый фикс — отдельный коммит, красные тесты подтверждались на старом коде (stash-проверки).

| Находка | Ключевые новые тесты | Статус |
|---|---|---|
| R01 admin-fallback | Alice≠admin матрица, пустой admin-список = запрет | ✅ |
| R02 Rust proxy | counting-proxy, malformed URL = ошибка конфигурации | ✅ |
| R03 stream без usage | always-count, estimate + `usage_source`, `include_usage`, genuine disconnect (raw ASGI / Drop-guard) | ✅ |
| R04 пустой choices | usage-чанк `choices: []`, null/missing choices, multi-choice | ✅ |
| R05 TOCTOU квот | admit/settle идемпотентность, 20 конкурентных → 1 upstream (обе ветки), refund при ошибке провайдера | ✅ |
| R06 routing без расхода | порог → backup (JSON + stream), `state_store=redis` запрещён | ✅ |
| R07 потеря параметров | forward `temperature/top_p/max_tokens/stop/name/tool_calls`, 422 на диапазоны, `metadata` не уходит наружу | ✅ |
| R08 Dockerfile | `COPY agent`, `--locked`, `.dockerignore` | ✅ (сборка) |
| R09 handshake агента | заголовки handshake unit + `real_tunnel_test` на настоящих бинарниках | ✅ |
| R10 listener :8443 | только `/ingress` на агентском порту, custom listen, disabled молчит | ✅ |
| R11 fail-closed | `Disabled/Ready/Unavailable`, malformed URL = panic, fail-closed при пустых квотах | ✅ |
| R12 stream audit | ровно одна строка: ok/block/rate_limited/upstream_error/cancelled + `request_id` (обе ветки) | ✅ |
| R13 restore на чанках | токен поперёк чанков, mask/restore политики, flush без потерь | ✅ |
| R14 CI | split workflows, fakeredis/enterprise в dev-deps, ruff/mypy/clippy в ноль | ✅ (gates) |
| R15 prometheus/auth | metrics-токены (200/401/403), service DNS + bearer_token_file, OpenAPI | ✅ (код; живой `up=1` — см. план выпуска) |
| R16 Helm PVC | PVC-шаблон, existingClaim/existingSecret, lint + template ×4 + kubeconform; живая установка отложена | ✅ частично (код; кластер — вне релиза) |

Gates на релизном срезе: Python 182 passed, Rust 81 passed, `ruff`/`mypy` чисто, `clippy --locked -D warnings` чисто (флаги CI).

---

## 2. Расшифровка статусов

* **401 `authentication_error`** — отсутствие/невалидный `Bearer` клиента; без `clients` — отклоняются все.
* **422 `invalid_request_error`** — `model`/`messages` не прошли валидацию Pydantic/serde, `code=invalid_body`.
* **403 `permission_error`** — `BlockedError` из инспектора (`reason=injection/dlp/blocked_source`), DLP `block` при наличии PII, инъекция при `severity >= block_severity_gte`.
* **429 `rate_limit_error`** — `QuotaExceeded` из metering (`scope=tokens/requests`, `limit` из `quotas.client_*_per_day`).
* **502 `upstream_error`** — `ProviderError` (HTTP статус провайдера или `Connection`), в стриме — `data: {"error":...}`.
* **Аудит (R12):** каждый запрос — ровно одна финальная строка с `request_id`; коды `ok/blocked/blocked_source/rate_limited/backend_error/upstream_error/cancelled` (disconnect — `cancelled`, mid-stream ошибка никогда не `ok`); `usage_source: upstream/estimated`.

## 3. Бенчмарк (честный прогон, mock-llm)

`scripts/bench.py` 50 rps × 10s, `fwllm-bench.yaml` (mock provider, DLP `off`):

|  | Python | Rust |
|---|---|---|
| `/healthz` | 7 ms p50 | 6 ms p50 |
| `chat` (mock 200) | 15 ms p50 | 10 ms p50 |
| Память | 2.6 GiB (cpython + LightAnon) | 1.4 MiB |

С ML-моделью XLM-R (1.1 GiB) Python ~2.85 GiB, инференс ~10 мс (локально) / ~80 мс на старом CPU без AVX2.

## 4. Вывод

* TDD-пирамида соблюдена: unit → integration (respx/fakeredis) → contract → live smoke.
* Каждый модуль имеет негативные и позитивные кейсы, включая краевые (пустые `providers`, истёкший `vault`, `stream` без `usage`).
* Rust-ветка повторяет контракты Python: один `openapi.yaml`/`policies.schema.json` для обеих веток.

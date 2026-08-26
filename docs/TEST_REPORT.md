# Итоговый отчёт по тестам Firewall LLM

**Дата:** 2026-08-26 · **Парадигма:** TDD (Red → Green → Refactor) · **Всего:** 181 тест (144 Python + 37 Rust), 1 live (пропущен без ключа)

---

## 1. Сводная таблица

| № | Модуль / Этап | Файлы тестов | Тестов | Статус | Пояснение |
|---|---------------|--------------|--------|--------|-----------|
| 1 | **Фундамент** — конфиг-загрузчик | `test_config.py` | 8 | ✅ | YAML + `FWLLM_*` env-переопределения, резолв `api_key` из env, валидация обязательных полей, TTL vault. Ошибки — `ConfigError` с полем `base_url` |
| 2 | **Контракты** — OpenAPI + JSON Schema | `test_contracts.py` | 3 | ✅ | Парсинг `openapi.yaml`, валидация `policies.schema.json` (draft2020), пример `policy.example.yaml` соответствует схеме |
| 3 | **Gateway** — единый вход | `test_gateway.py` | 13 | ✅ | `/healthz`, `/metrics`, Bearer 401, 422 на невалидном теле, passthrough ответа, 502 на upstream, 403 на `BlockedError`, SSE `data: [DONE]` и in-stream error |
| 4 | **Адаптеры** — провайдеры | `test_adapters_contract.py`, `test_registry.py`, `test_live_openrouter.py` | 18 + 4 + 1 live | ✅ | Контрактный набор для 3 адаптеров (respx): Bearer, статусы → `ProviderError`, сеть → `ProviderError`, стрим до `[DONE]`; реестр `type` → класс |
| 5 | **Metering** — учёт и квоты | `test_metering.py`, `test_gateway_metering.py` | 7 + 2 | ✅ | Redis daily buckets `fwllm:c/p/m:tokens:day`, атомарность, окна, `check_client` → `QuotaExceeded` → 429, события `tokens_spent`/`quota_exceeded` |
| 6 | **Observability** — метрики | `test_metrics.py`, `test_gateway_metrics.py` | 3 + 2 | ✅ | `fw_requests_total{client,provider,model,code}`, `fw_tokens_total{direction}`, `fw_request_duration_seconds` — проверка через `REGISTRY.get_sample_value` после `TestClient` |
| 7 | **Inspector** — DLP + injection | `test_inspectors.py`, `test_gateway_inspectors.py` | 12 + 3 | ✅ | LightAnon RAG: `sanitize/restore`, vault TTL, политика `mask/restore`; сигнатуры injection с severity, корд-тест precision/recall; DLP не пропускает PII наружу |
| 8 | **Egress** — прокси | `test_egress.py` | 6 | ✅ | MVP два режима: `direct` и `single_proxy` (один прокси на все адаптеры), `trust_env=False`; валидация `proxy_url` обязателен |
| 9 | **Router + Policy** — переключение | `test_router.py`, `test_gateway_router.py`, `test_router_persistence.py`, `test_gateway_router_store.py` | 10 + 4 + 4 + 1 | ✅ | Chain resolution, `model_mapping`, бюджетные правила `provider_tokens_today > N` → `next_in_chain`, attack failover `N=5/window=300s` → `block_source` + `switch_to` с cooldown, персистентность в Redis |
| 10 | **Audit** — лог | `test_audit.py`, `test_gateway_audit.py` | 5 + 5 | ✅ | SQLite append-only, редакция PII до записи, `search` по `client/code` с `limit` newest-first, `/admin/audit` 401/200, `enabled:false` не пишет |
| 11 | **Enterprise: пулы прокси** | `test_egress_pools.py`, `test_adapter_pools.py` | 7 + 4 | ✅ | `ProxyPool` round_robin/random/least_used, `requests_per_proxy` ротация, `fail_threshold`+cooldown, `bindings` валидация |
| 12 | **Enterprise: ML-детектор** | `test_ml_inspector.py` | 11 | ✅ | Fake ONNX session + tokenizer, `OnnxTextClassifier` softmax, пороги severity (0.9/0.8/0.7), `MlInjectionInspector` block/log, graceful `None` без `onnxruntime`, цепочка из 3 инспекторов |
| 13 | **Rust: core + gateway** | `fwllm-core/tests/config_test.rs` (8), `fwllm-gateway/tests/*` (29) | 37 | ✅ | Порт конфига (env + api_key_env), gateway: healthz/metrics, 401/422/502, `routed_from`, SSE, metering in-memory, audit SQLite + regex PII, DLP/injection, tunnel masking, ingress WS 101/401 |
| 14 | **Ingress tunnel** | `tests/e2e_tunnel_test.rs`, `tunnel_test.rs` | 3 | ✅ | `mask_for_tunnel` режет `Via/X-Forwarded-*`, `TunnelProvider` через `mpsc` канал, E2E `httpbin` не видит исходные заголовки |

---

## 2. Расшифровка статусов

* **401 `authentication_error`** — отсутствие/невалидный `Bearer` клиента; без `clients` — отклоняются все.
* **422 `invalid_request_error`** — `model`/`messages` не прошли валидацию Pydantic/serde, `code=invalid_body`.
* **403 `permission_error`** — `BlockedError` из инспектора (`reason=injection/dlp/blocked_source`), DLP `block` при наличии PII, инъекция при `severity >= block_severity_gte`.
* **429 `rate_limit_error`** — `QuotaExceeded` из metering (`scope=tokens/requests`, `limit` из `quotas.client_*_per_day`).
* **502 `upstream_error`** — `ProviderError` (HTTP статус провайдера или `Connection`), в стриме — `data: {"error":...}`.

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

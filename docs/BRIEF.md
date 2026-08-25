# Бриф проекта: Firewall LLM (fwllm)

| | |
|---|---|
| **Версия документа** | 1.0 |
| **Дата** | 2026-08-25 |
| **Статус проекта** | MVP в разработке |
| **Модель разработки** | Open-core, TDD |

---

## 1. Концепция

**Firewall LLM (fwllm)** — шлюз безопасности (security gateway) для трафика к большим
языковым моделям и AI-агентам. Продукт устанавливается on-prem, встаёт между
приложениями и LLM-провайдерами и контролирует:

- **Расходы на токены** — учёт, квоты, лимиты per user/team/project;
- **Утечки данных (DLP)** — обнаружение и маскирование PII и чувствительных данных;
- **Prompt injection / jailbreak** — детекция атак на модели;
- **Теневой ИИ** — блокировка неавторизованных провайдеров и endpoints;
- **Маршрутизацию моделей** — несколько провайдеров за единым API с автопереключением.

Аналоги на рынке: Lakera Guard, Protect AI, Prompt Security, Azure AI Content Safety.
Отличие — open-core модель и полноценный on-prem/air-gapped режим.

## 2. Цели и задачи MVP

1. Единый OpenAI-совместимый вход (`POST /v1/chat/completions`) для всех клиентов.
2. Сменные адаптеры провайдеров: по одному внешнему API — свой прокси-адаптер.
3. Учёт токенов и запросов, квоты, алерты.
4. DLP-маскирование исходящих промптов на базе LightAnon (обратимая санитизация).
5. Детектор prompt injection (сигнатуры + события).
6. Политики маршрутизации в YAML: переключение провайдера по количеству токенов,
   количеству запросов, при детекции атаки, при деградации провайдера.
7. Пул исходящих HTTP-прокси, назначаемый на каждый адаптер, с ротацией по
   настраиваемому количеству запросов.
8. Метрики Prometheus + дашборд в существующем инстансе Grafana (импорт через API).
9. Аудит-лог всех запросов/ответов с редакцией PII.

## 3. Ключевые архитектурные решения

### 3.1 Модульная архитектура

```
Клиенты ──► Unified API ──► CORE Gateway
                              ├─ Router (+ Policy Engine)
                              ├─ Inspector (prompt injection, DLP/LightAnon)
                              ├─ Metering (токены, квоты)
                              ├─ Egress Proxy Pool
                              ├─ Audit Log
                              └─ Observability (/metrics → Prometheus → Grafana)
                                    │
                    Provider Adapters: [openrouter] [openai] [anthropic] [ollama]
```

Единая точка входа; несколько моделей за одним API. Каждый внешний API обслуживается
своим адаптером. Роутер выбирает адаптер по политикам.

### 3.2 Политики переключения провайдеров

Декларативные YAML-правила, движок один — триггеры любые. Событийная модель:
metering/inspector публикуют события (`tokens_spent`, `requests_count`,
`attack_detected`, `provider_error`) → policy engine пересчитывает routing table →
роутер читает актуальное состояние.

Примеры правил:

```yaml
rules:
  - name: token-budget-switch          # по количеству токенов
    when: { provider_tokens_today: { gt: 5000000 } }
    action: switch_to: backup-anthropic

  - name: rate-switch                  # по количеству запросов
    when: { requests_per_minute: { gt: 500 } }
    action: switch_to: local-vllm

  - name: attack-failover              # при детекции атаки
    when: { event: attack_detected, severity: { gte: high }, count: { gt: 3 } }
    action: { block_source: true, switch_default_to: local-vllm }

  - name: health-failover
    when: { provider_error_rate: { gt: 0.1 }, window: 60s }
    action: switch_to: next_in_chain
```

Состояние переключения персистентно (Redis), переживает рестарт, имеет cooldown и
ручной сброс. Маппинг моделей между провайдерами (например, `gpt-4o` ↔ free-модель
OpenRouter) — через таблицу соответствий.

### 3.3 Интеграция LightAnon

Библиотека [LightAnon](https://github.com/SoldatovAlexander/lightanon_project)
(MIT) используется как DLP-движок в цепочке инспекторов:

- request: `rag sanitize` — замена чувствительных данных токенами перед отправкой;
- response: `rag restore` — обратное восстановление из ответа LLM (политика
  `restore` или `mask` выносится в YAML);
- vault хранится локально с TTL (on-prem требование);
- финансовые правила, профиль соответствия `ru_152`, compliance-отчёты.

### 3.4 Egress proxy pool

Пул исходящих HTTP(S CONNECT) прокси, назначаемых на каждый адаптер провайдера:

```yaml
egress_proxies:
  pools:
    openai-pool:
      proxies: ["http://proxy1.internal:8080", "http://proxy2.internal:8080"]
      rotation: { strategy: round_robin, requests_per_proxy: 200 }
      healthcheck: { interval: 60s, fail_threshold: 3, cooldown: 5m }
  bindings:
    openai-adapter: openai-pool
```

Стратегии ротации: round_robin / random / least_used; переключение по настраиваемому
количеству запросов; healthcheck с исключением и cooldown. События пула
(`proxy_rotated`, `proxy_failed`) попадают в policy engine и метрики.

### 3.5 Наблюдаемость

- `prometheus_client`, endpoint `/metrics`;
- метрики с лейблами `{provider, adapter, model, proxy, route}`:
  `fw_requests_total`, `fw_tokens_total{direction}`, `fw_request_duration_seconds`,
  `fw_injection_detections_total{severity}`, `fw_dlp_redactions_total{rule}`,
  `fw_router_active_provider`, `fw_proxy_requests_total{proxy}`, `fw_proxy_healthy`.
- Дашборды — JSON в репо (`deploy/grafana/dashboards/`), идемпотентный импорт в
  существующий инстанс Grafana через HTTP API (обновление по UID).

### 3.6 On-prem требования

- Никакой телеметрии наружу; Redis/Postgres внутри контура;
- Локальные ONNX-модели детекторов, offline-обновление сигнатур;
- Аутентификация админки через LDAP/OIDC существующего контура;
- Деплой Docker Compose / Helm, air-gapped установка.

## 4. Технологический стек

| Компонент | Выбор |
|---|---|
| MVP-ветка | **Python 3.11+, FastAPI, httpx, redis, prometheus-client, lightanon** |
| Вторая ветка | **Rust** (production gateway: axum/hyper, низкие задержки; контракты общие с Python) |
| Контракты | OpenAPI unified API + JSON Schema политик (единые для обеих веток) |
| Хранилища | PostgreSQL (логи, состояние), Redis (квоты, routing state) |
| Наблюдаемость | Prometheus + Grafana (существующий инстанс) |
| Тесты | pytest, respx, testcontainers, schemathesis; парадигма **TDD** |

## 5. Open-core модель

**Открытая часть**: gateway, unified API, адаптеры, router, policy engine, базовый
metering, правила DLP/injection, audit log, CLI.

**Проприетарная часть (Enterprise)**: обученные ML-детекторы инъекций, продвинутый
DLP (NER, классификация данных), UI управления, SIEM-интеграции, RBAC/LDAP,
мультиарендность, HA-кластер, air-gapped обновления.

Ядро определяет публичные extension points (`Inspector`, `AuthProvider`,
`AuditSink`) — enterprise-фичи и сторонние плагины подключаются как плагины.

Лицензия ядра: **к выбору** — FSL/BSL (рекомендуется) или AGPL-3.0. Решение должно
быть принято до публичного пуша.

## 6. План реализации (TDD)

Полный план — `docs/PLAN.md`. Принципы: red→green→refactor; контракты раньше кода;
пирамида тестов (unit → integration → e2e → contract); секреты только в `.env`.

| Фаза | Содержание | Срок |
|---|---|---|
| 0 | Фундамент: скелет пакета, CI, контракты, конфиг | 0.5 нед |
| 1 | Gateway: unified API, streaming, аутентификация | 1 нед |
| 2 | Адаптеры: Provider protocol, OpenRouter (free models), Ollama | 1 нед |
| 3 | Metering: счётчики, квоты, события | 1 нед |
| 4 | Observability: метрики, импорт дашборда в Grafana | 0.5 нед |
| 5 | Inspector: DLP/LightAnon + prompt injection | 1.5 нед |
| 6 | Egress proxy pool: пул, ротация, healthcheck | 1 нед |
| 7 | Router + Policy Engine: правила, переключения, персистентность | 1.5 нед |
| 8 | Audit log + Admin API | 1 нед |
| 9 | Стабилизация: нагрузочное, Compose/Helm, документация | ~1 нед |

**Вехи:**

- **M1** — рабочий прокси к бесплатным моделям (фазы 0–2)
- **M2** — учёт токенов + дашборд в Grafana (фазы 3–4)
- **M3** — безопасность (DLP/injection) + прокси-пул (фазы 5–6)
- **M4** — автопереключение провайдеров + аудит; MVP закрыт (фазы 7–8)

## 7. Инфраструктура

| Ресурс | Назначение |
|---|---|
| dev-server `192.168.88.101` (Ubuntu 24.04, Docker 29) | Тестовый стенд: Prometheus, Grafana, Redis, mock-провайдер |
| GitHub (репозиторий создаётся) | Хостинг кода, CI |
| OpenRouter test key | Бесплатные модели для разработки и smoke-тестов |

## 8. Риски и открытые вопросы

| Риск/вопрос | Митигация/статус |
|---|---|
| Расхождение Python/Rust веток | Единые контракты + контрактные тесты до реализации |
| Vault LightAnon при стриминге | Отдельная задача фазы 5: чанковый restore |
| Прокси без HTTPS CONNECT | Проверка поддержки на этапе healthcheck |
| Лицензия не выбрана | Решить до первого публичного push (FSL vs AGPL) |
| Ключи/пароли в репо | `.env` в `.gitignore`, `.env.example` вместо реальных значений |

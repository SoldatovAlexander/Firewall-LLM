# План: Ingress Tunnel Proxy — Фаза 10

**Статус:** draft · **Дата:** 2026-08-26 · **Порт входящих туннелей:** `8443` · **TLS:** self-signed CA

---

## 1. Контекст и цель

Компании не могут отправлять запросы к LLM API напрямую. Нужна маскировка окружения и точки входа.

Расширение: **удалённый агент-прокси** поднимается на разрешённом хосте. На шлюзе создаётся токен, вводится в агент — поднимается `wss://gateway:8443/ingress` туннель. Шлюз отправляет уже готовые HTTP-пакеты (метод + URL + заголовки + body с точкой назначения внутри), агент **только маскирует** (`Via`, `X-Forwarded-*`, `X-Real-IP`, `Server` → generic `User-Agent`) и форвардит к `Destination`, возвращает ответ.

Остальная логика (DLP, injection, квоты, аудит) уже на шлюзе — агент максимально тупой и лёгкий.

## 2. Архитектура

```
Приложение → Firewall LLM Gateway (:8080) → wss:8443 → Agent (прокси) → https://api.llm-provider.com/v1
                 │ DLP/квоты/аудит                │ маскировка
                 └────────────────────────────────┘
                          self-signed TLS (ca.crt)
```

**Компоненты**

* **Gateway `rust/fwllm-gateway/src/ingress`** — TLS listener `:8443` (rustls, `ca.crt/server.crt`), `POST /admin/ingress/tokens`, `GET /admin/ingress/agents`, `GET /ingress` (WebSocket upgrade, `Authorization: Bearer <token>`), реестр агентов (`agent_id → WS Sender`), heartbeat `15с`.
* **Agent `rust/fwllm-agent`** — статический бинарь Rust (~7 МБ), `systemd`/`Docker`, `wss` клиент с `ca.crt`, реконнект с backoff, форвард фреймов через `reqwest`.

## 3. Протокол туннеля (v1)

Фреймы — JSON по WebSocket `Text`.

`Gateway → Agent` (запрос):
```json
{"id":"uuid","method":"POST","url":"https://api.example.com/v1/chat/completions","headers":{"content-type":"application/json"},"body":"base64?"}
```

`Agent → Gateway` (ответ):
```json
{"id":"uuid","status":200,"headers":{"content-type":"application/json"},"body":"base64?"}
```

Маскировка на агенте (до `fetch`):
* удалить `via`, `x-forwarded-*`, `x-real-ip`, `server`, `cf-*`
* `user-agent` → `Firewall-LLM-Agent/0.1` если отсутствует
* не трогать `authorization`, `content-type`

Heartbeat: `{"type":"ping"}` / `{"type":"pong"}` каждые 15с, таймаут 45с.

## 4. Конфигурация

`fwllm.yaml` (шлюз):
```yaml
ingress:
  enabled: true
  listen: "0.0.0.0:8443"
  cert: "./certs/server.crt"   # генерится при первом старте, если нет
  key: "./certs/server.key"
  ca: "./certs/ca.crt"
```

`agent.yaml` (на удалённом хосте):
```yaml
gateway_url: wss://fwllm.internal:8443/ingress
token: "<из POST /admin/ingress/tokens>"
ca_cert: /etc/fwllm/ca.crt
```

Токен: `43` символа `Alphanumeric`, хранится хешем в `IngressRegistry`, TTL по умолчанию `168ч` (передается в `POST {agent_id, ttl_hours}`).

## 5. Задачи и TDD-тесты (Red → Green)

| Этап | Задача | Тесты первыми (Red) |
|------|--------|---------------------|
| **10.1** | `IngressRegistry` — выпуск/валидация/истечение токенов, список агентов | `cargo test ingress_registry_*` — токен создаётся, валиден, истекает; `validate_token` возвращает `None` для чужого/просроченного |
| **10.2** | `POST /admin/ingress/tokens` + `GET /admin/ingress/agents` (bearer auth как у `/v1/*`) | `ingress_test.rs`: `401` без токена шлюза, `200` создаёт токен для `agent_id`, `GET` возвращает список |
| **10.3** | TLS `:8443` self-signed + `GET /ingress` WS upgrade с `Bearer <ingress-token>` | `ws_handshake_requires_valid_token` — `101` с валидным, `401` с невалидным/просроченным |
| **10.4** | `fwllm-agent` — `wss` клиент, загрузка `ca.crt`, реконнект | `cargo test -p fwllm-agent` — `load_ca` читает `ca.crt`, `mask_headers` режет `Via`/`X-Forwarded` |
| **10.5** | Форвард фреймов: Gateway → Agent → Destination (`httpbin` в тесте) | `e2e_tunnel_masks_headers` — шлюз шлёт `X-Forwarded-For: 1.1.1.1` через туннель, `httpbin` получает без него |
| **10.6** | Интеграция в `egress.mode: tunnel` — провайдер шлюза идёт через туннель вместо `reqwest` напрямую | `tunnel_provider_uses_agent` — `POST /v1/chat/completions` с `model: via-tunnel` уходит через агента |

## 6. Критерии готовности (DoD)

* `POST /admin/ingress/tokens` + `GET /admin/ingress/agents` работают, токены истекают
* `wss://gateway:8443/ingress` требует валидный `Bearer`, self-signed `ca.crt` раздаётся инсталлятором
* Агент ≤10 МБ RAM, реконнект с backoff, маскировка проверена `httpbin`
* E2E: запрос к LLM через шлюз уходит через агента, `Destination` не видит исходный `X-Forwarded-*`
* `cargo test --workspace` зелёный, `cargo clippy -- -D warnings` чистый

## 7. Коммиты

`feat(ingress): registry + token issuance`
`feat(ingress): TLS :8443 + wss handshake`
`feat(agent): fwllm-agent binary + header masking`
`feat(ingress): tunnel egress provider`
`docs: ingress protocol + deploy`

## 8. Риски и митигация

* Самоподписанный CA — дистрибуция `ca.crt` только вместе с токеном (не в git), инсталлятор `install-agent.sh` кладёт оба в `/etc/fwllm/0600`
* Старый CPU без AVX2 (как на dev-server) — агент без `polars`/`onnx`, только `reqwest` — проблемы нет
* Потеря `ca.key` — бэкап `./certs/` в `deploy/certs/` (gitignored)

## 9. Деплой dev-server

```bash
cd deploy
./gen-certs.sh  # ca.crt/server.crt/key -> ./certs/ (если нет)
docker compose up -d --build gateway  # пробрасывает 8443:8443 + ./certs:/certs:ro + ./models
curl -k https://192.168.88.101:8443/healthz  # проверка TLS
```

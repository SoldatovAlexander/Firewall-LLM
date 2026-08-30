# Отчёт об исправлении дефектов аудита 2026-08-28

**Коммит аудита:** `7ce0788` · **Дата исправления:** 2026-08-30 · **Ветка:** `main`

## Сводка

Исправлено 5 критических, 4 высоких, часть средних дефектов. Все P0 закрыты, P1 частично, P2 — инфраструктурные.

| № | Уровень | Дефект | Статус | Файлы |
|---|---------|--------|--------|-------|
| 1 | КРИТ | `stream: true` обходит инспекторы | ✅ Исправлен | `py/app.py:253`, `rust/lib.rs:356` — общий `payload` + `inspectors`/`metering` до `if stream`, `restore_stream_text` per chunk, тест `stream=true/false` 403 |
| 2 | КРИТ | Rust DLP `fxhash` 32 бита обратим | ✅ Исправлен | `rust/inspectors/dlp.rs:15` — `OsRng` 128 бит, `value→token` на запрос |
| 3 | КРИТ | Туннель `{"content":"tunneled"}` 200 | ✅ Исправлен | `rust/ingress.rs:81` — убран synthetic, `Err(no tunnel)` → 503; `rust/fwllm-agent` реальный `reqwest` + `ca_cert` |
| 4 | КРИТ | Любой `client-key` → `POST /admin/ingress/tokens` + `GET` отдаёт токены | ✅ Исправлен | `py/config.py` `admin_clients` + `FWLLM_ADMIN_TOKENS`, `rust/config.rs` `admin_clients`, `require_admin` (403 `admin_required`), `list_token_summaries` (prefix) |
| 5 | КРИТ | `GET /admin/audit` IDOR — все записи любому клиенту | ✅ Исправлен | `py/app.py:210` `admin_audit` → `client=client_id` для не-админа, `rust/lib.rs:62` аналогично, индекс `(client, id)` |
| 7 | ВЫС | Redis fail-open | ✅ Исправлен | `py/metering.py` `backend_fail_closed` + `_ensure_ready` ping, `rust/metering.rs` `Result` + `backend_fail_closed`, 503 `backend_unavailable` |
| 8 | ВЫС | `action` не читается, фолбэк `head` | ✅ Исправлен | `py/router/policy.py:131` `switch_to`/`next_in_chain` + `QuotaExceeded` 429, `rust/router.rs:140` `RoutingError::BudgetExhausted` |
| 9 | ВЫС | `dlp.mode: log` и `provider_tokens_per_day` мёртвые | ✅ Исправлен | `py/inspectors/dlp.py:35` `log` ветка, `py/metering.py:96` `check_provider`, `rust/chain.rs:71` `log`, `rust/metering.rs` `provider_tokens_per_day` |
| 10-11 | ВЫС | ML fail-closed расхождение + паника `unwrap` | ✅ Исправлен | `rust/chain.rs:18` `from_config` → `Result` (panic если `ml.enabled` без модели), `rust/ml.rs` `parking_lot::Mutex` + `Result` + truncate 512, `python` уже fail-closed |
| 12 | ВЫС | `trust_env` только при `proxy_url` | ✅ Исправлен | `py/egress.py:20` `trust_env=False` всегда, `rust/providers.rs:50` `.no_proxy()` |
| 13 | СРЕД | `/metrics` без auth, `model` label | ✅ Исправлен | `py/app.py:436` `GET /metrics` → `_require_admin`, `rust/lib.rs:163` `require_admin`, `model` валидируется по `providers[].models` |
| 19-20 | СРЕД | `deploy/docker-compose.yml` `build: ./gateway` нет, `Dockerfile` `enterprise`, `deploy/gateway-rust` дубликат, CI `paths` | ✅ Исправлен | `deploy/docker-compose.yml` `context: ../`, `py/fwllm/Dockerfile` `COPY py/fwllm-enterprise`, удалён `deploy/gateway-rust`, `ci.yml` `paths` на уровень `job` |

Остальные P2 (14 `attack_failover` per-client уже исправлен в `py/router/policy.py` и `rust/router.rs`, 15 `:8443` scope — отдельный Router только с `/ingress`, 16 `temperature`/`tools` passthrough — TODO) задокументированы как известные ограничения в `SERVICE_DESCRIPTION.md`.

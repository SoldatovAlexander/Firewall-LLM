# План исправлений по релизному ревью 2026-09-06

Источник: `docs/RELEASE_CODE_REVIEW_2026-09-06.md` (ревизия `41fd102`, 13×P1 + 3×P2).
Вердикт ревью: **production-релиз отложить** до закрытия P1.
Старый план `PLAN_FIX_AUDIT.md` (аудит 28.08) частично выполнен, часть дефектов
переоткрыта в новом ревью — ниже актуальная сверка.

## Сверка со старым планом

| Новое | Статус старого | Комментарий |
|---|---|---|
| R01 admin-fallback | было №4, **переоткрыто** | fallback «любой клиент = админ при пустом списке» оставлен сознательно — ревью требует убрать: пустой admin-список = запрет |
| R03 stream без usage | было №1, **недоделано** | инспекторы вынесены, но учёт только при ненулевом usage; нужен always-count + `stream_options.include_usage` |
| R11 fail-closed обход | было №7, **недоделано** | `RedisStore::new(...).ok()` глотает ошибку URL; нужен `Disabled/Ready/Unavailable` |
| R06 routing без расхода | **новое** | `record_tokens` никто не вызывает; Rust игнорирует `state_store: redis` |
| R09 handshake агента | было №3, **недоделано** | реальный бинарь падает: нет `sec-websocket-key` (`IntoClientRequest`) |
| R13 restore на чанках | **новое** | нужен stateful stream-decoder с буфером префикса |
| R14 CI | было №20, **переоткрыто** | `paths: rust/**` глушит Python; нет `fakeredis`/`enterprise` в dev-deps; ruff 15 + mypy 1 + clippy 6 |
| R08 Dockerfile | было №19, **переоткрыто** | workspace включает `fwllm-agent`, COPY его нет |
| R02 Rust proxy | **новое** | `egress.mode/proxy_url` не участвуют, всегда `.no_proxy()` |
| R04 пустой choices | **новое** | `IndexError`, оборванный SSE вместо 502 |
| R05 TOCTOU квот | **новое** | check-then-increment; нужен Lua reserve/settle |
| R07 потеря параметров | было №16 частично | `temperature/max_tokens/stop/metadata/tools` теряются |
| R10 listener :8443 | **новое** | `app.clone()` на :8443 отдаёт весь API; нет `IngressConfig` |
| R12 Rust stream audit | **новое** | `stream_response` без `AuditLog` |
| R15 prometheus/auth | **новое** | scrape на LAN IP без токена; Rust target отсутствует |
| R16 Helm PVC | **новое** | `persistence.enabled` требует несуществующий PVC; `existingSecret` не подключён |

## Этапы (порядок = зависимости, TDD, коммит после каждого)

| Этап / PR | Содержание | Оценка | Готовность |
|---|---|---:|---|
| 1. Сборка и gates | R08, R14: Dockerfile (COPY agent, `--locked`, `.dockerignore`), CI triggers, dev-deps, ruff/mypy/clippy в ноль | 1–2д | чистый checkout собирается, gates зелёные |
| 2. Граница доверия | R01, R02, R10, R11: убрать admin-fallback, `reqwest::Proxy` по `EgressConfig`, отдельный ingress-Router + `IngressConfig`, `Disabled/Ready/Unavailable` для metering | 2–4д | negative-тесты: Alice≠admin, proxy обязателен, :8443 только `/ingress`, malformed URL = ошибка конфигурации |
| 3. API и streaming | R04, R07, R13: `chunk["choices"] or []`, DTO по `openapi.yaml` (stop/metadata/tools инспектируются), stateful restore-decoder | 2–4д | общий корпус проходит на обеих ветках |
| 4. Расходы и routing | R03, R05, R06: always-count + `include_usage` + estimate, Lua reserve/settle, `record_tokens` в путь, `RouterStateStore`/запрет `state_store=redis` в Rust до реализации | 3–5д | конкурентный тест 20→1, рестарт/реплика видят бюджеты |
| 5. Туннель и аудит | R09, R12 + lifecycle: `IntoClientRequest` + Authorization, настоящий WSS exchange, audit-finalizer обеих веток | 2–4д | бинарники выполняют exchange; все terminal outcomes в аудите |
| 6. Поставка и RC | R15, R16, `.env.example`, профили: service DNS + credentials file, PVC/existingSecret, smoke | 2–3д | Compose/Helm smoke, `up=1`, доки согласованы |

Итого **12–22 инженерных дня**. P0 не назначен. P1 — до production-релиза компонента, P2 — до релиза возможности либо явное исключение из профиля поставки.

## Минимальные условия выпуска (из ревью, без изменений)

- Все P1 закрыты проверками на точном release commit.
- JSON и SSE: success/block/quota/error/timeout/disconnect — аудит и учёт согласованы.
- Чистая сборка, настоящий WSS, обязательный proxy, отказ Redis в fail-closed, конкурентные квоты, рестарт.
- Матрица Python/Rust опубликована; неподдерживаемое — с явным отказом конфигурации.

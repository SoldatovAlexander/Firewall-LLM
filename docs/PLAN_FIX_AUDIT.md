# План исправления по аудиту 2026-08-28 (коммит 7ce0788)

Источник: `Технический аудит открытого репозитория` — 23 дефекта (5 критических). Оценка по влиянию на 4 заявленные функции.

## Порядок (как в отчёте, § «Что чинить в первую очередь»)

**P0 — без этих мер остальная защита обходится:**
1. **№1** `stream: true` — обход инспекторов/квот. Вынести `inspectors/metering/router` до `if stream`, потоковая инспекция (`restore_stream_text`), `usage` из последнего чанка, контрактный тест `stream=true/false` одинаковый вердикт.
2. **№3** Synthetic туннель. Убрать литерал `tunneled` (200), при отсутствии канала — `503`, реализовать `register_tunnel` + `ProxyRequest`→`oneshot`, реальный `reqwest` в агенте + `ca_cert`.
3. **№4** Любой `client-key` → `POST /admin/ingress/tokens` + `GET` отдаёт токены. Ввести `admin_tokens` + `require_admin` на все `/admin/*`, скоупы `chat/audit/ingress`, хранить хэши, в листинге — `agent_id/prefix/expires_at`.
4. **№2** Rust DLP `fxhash` 32 бита → случайные токены `OsRng` 128 бит, словарь `value→token` на запрос.
5. **№5** `GET /admin/audit` IDOR — жёстко `search(client=client_id)` в Python, игнор `?client=` в Rust, роль `auditor/admin`, индекс `(client, id)`.

**P1 — тихая деградация:**
6. №7 `fail-open` Redis → `metering.on_backend_error: reject|allow` (по умолчанию `reject`), `deadpool-redis`, `Result` вместо `0`, `fw_metering_degraded` + `/healthz degraded`, резервирование бюджета Lua.
7. №8 `action` не читается — реализовать `next_in_chain`/`switch_to`, при исчерпании всех кандидатов — `429/503` или `routing.on_all_exhausted`.
8. №9 `dlp.mode: log` и `provider_tokens_per_day` — реализовать или валидатор отвергает.
9. №10 Расхождение `ml.enabled` — fail-closed в обеих ветках или `ml.required: false` + метрика.
10. №11 Паника ML → `parking_lot::Mutex`, `spawn_blocking`, `CatchPanicLayer`, `Result`.

**P2 — поставка/CI:**
11. №19 `deploy/docker-compose.yml:8` (`./gateway` нет), `py/fwllm/Dockerfile:11` (`enterprise`), `.env.example` нет — исправить `build.context`, добавить `.env.example`, `USER`, `HEALTHCHECK`.
12. №20 `ci.yml:3` `paths: ["rust/**"]` глушит Python — разнести `paths` на уровень `job`, удалить `deploy/gateway-rust` дубликат.

Каждый этап: Red → Green → `git commit` (префикс `fix(audit-N):`), в конце `push`.

## DoD этапа
* Контрактный тест фазы зелёный, `cargo test` + `pytest` зелёные, `clippy -D warnings`/`ruff`/`mypy` чисто.
* Дефект не воспроизводится PoC из отчёта.

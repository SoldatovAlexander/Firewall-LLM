# Firewall LLM — Документация (RU)

- [Шлюз](gateway.md) — единый OpenAI-совместимый API, аутентификация, ошибки, стриминг
- [Адаптеры](adapters.md) — реестр провайдеров, OpenRouter / Ollama / Tunnel
- [Маршрутизация](routing.md) — цепочки, маппинг моделей, бюджетные правила, failover атак
- [Инспекторы](inspectors.md) — DLP (LightAnon) + инъекции (сигнатуры + ML)
- [Учёт](metering.md) — счётчики, квоты, fail-open
- [Egress](egress.md) — direct / single_proxy / pools (enterprise) / tunnel
- [Аудит](audit.md) — SQLite, редакция PII, `/admin/audit`
- [Наблюдаемость](observability.md) — Prometheus `/metrics`, импорт в Grafana
- [Ingress-туннель](ingress.md) — `wss://:8443`, токены, агент, маскировка
- [Развёртывание](deployment.md) — Docker Compose, Helm, сертификаты

- [Простое описание сервиса](../../docs/SERVICE_DESCRIPTION_SIMPLE.md) — как работает с примерами

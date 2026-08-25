# Firewall LLM (fwllm)

Security gateway для LLM и AI-агентов: контроль запросов, учёт токенов, защита от
утечек данных, prompt injection, теневого ИИ. On-prem, open-core.

## Статус

MVP в разработке. Python-ветка.

## Архитектура (кратко)

- **gateway** — единый OpenAI-совместимый вход (`/v1/chat/completions`)
- **adapters** — сменные адаптеры провайдеров (OpenRouter/OpenAI/Anthropic/Ollama)
- **router + policy engine** — переключение провайдеров по политикам (токены, RPS, атаки)
- **inspector** — prompt injection + DLP-маскирование (LightAnon)
- **metering** — учёт токенов/запросов, квоты
- **egress** — пул исходящих прокси с ротацией по количеству запросов
- **observability** — Prometheus-метрики, импорт дашбордов в существующую Grafana

## Структура

```
contracts/    # OpenAPI unified API + JSON Schema политик (общие с Go-веткой)
py/fwllm/     # Python-MVP
deploy/       # docker-compose, Grafana dashboards
tests/
```

# Firewall LLM (fwllm)

Security gateway for LLM traffic: token spend control, data-leak prevention, prompt injection defense, shadow AI blocking.

![Python](https://img.shields.io/badge/python-3.11%2B-blue)
![Rust](https://img.shields.io/badge/rust-gateway-orange)
![License](https://img.shields.io/badge/license-FSL--1.1--MIT-blue)
![Deployment](https://img.shields.io/badge/deployment-on--prem-red)

## Project Description / Описание проекта

**EN:** Firewall LLM is an open-core security gateway that sits between applications and LLM providers. It exposes a single OpenAI-compatible API, routes requests to multiple models through swappable provider adapters, meters tokens and requests, masks sensitive data before it leaves the perimeter, detects prompt injection attacks, and switches providers automatically by policy. Fully on-prem: no telemetry leaves the perimeter.

**RU:** Firewall LLM — open-core шлюз безопасности между приложениями и LLM-провайдерами. Единый OpenAI-совместимый API, маршрутизация, учёт токенов, маскирование данных, детекция инъекций и автопереключение провайдеров по политикам. Полностью on-prem.

## Features
- Unified OpenAI-compatible API (`POST /v1/chat/completions`, streaming SSE)
- Provider adapters: OpenRouter / OpenAI / Ollama / Tunnel (via `wss://:8443` agent)
- Policy routing: token budgets, attack failover, model mapping
- Injection detection: signatures + local ONNX ML (severity-based)
- DLP masking via LightAnon (ru_152) — reversible
- Egress: direct / single_proxy (open) / pools + tunnel (enterprise)
- Metering quotas (Redis, fail-open) + Audit (SQLite, PII redaction)
- Prometheus `/metrics` + Grafana dashboard import
- Two branches, one contract: Python + Rust (axum)

## Architecture

```
Clients ──► Unified API ──► Gateway
                          ├─ Router + Policy Engine
                          ├─ Inspector (injection / DLP)
                          ├─ Metering / Audit / Metrics
                          └─ Egress → [openrouter | ollama | tunnel → Agent → LLM API]
```

See `docs/en/README.md` / `docs/ru/README.md` for module docs.

## Quick Start

```bash
git clone https://github.com/SoldatovAlexander/Firewall-LLM.git
cd Firewall-LLM/py/fwllm
python3 -m venv .venv && source .venv/bin/activate
pip install -e '.[dev]'
cp config.example.yaml fwllm.yaml && cp .env.example .env  # set OPENROUTER_API_KEY, FWLLM_CLIENT_TOKENS
FWLLM_CONFIG=./fwllm.yaml uvicorn fwllm.main:app --host 0.0.0.0 --port 8080
curl http://127.0.0.1:8080/v1/chat/completions -H "Authorization: Bearer <key>" -H "Content-Type: application/json" -d '{"model":"gpt-4o","messages":[{"role":"user","content":"Hello!"}]}'

# Rust (production)
cd ../../rust && FWLLM_CONFIG=../py/fwllm/fwllm.yaml cargo run -p fwllm-gateway
```

Full stack: `cd deploy && cp fwllm.yaml.example fwllm.yaml && cp .env.example .env && docker compose up -d --build` (gateway :8080, Rust :8081, ingress :8443, Grafana).

## Tests

```bash
cd py/fwllm && pytest -q && ruff check src tests && mypy src
cd ../../rust && cargo test --workspace && cargo clippy -- -D warnings
```

## Documentation

EN: [`docs/en/README.md`](docs/en/README.md) · RU: [`docs/ru/README.md`](docs/ru/README.md)

| Module | EN | RU |
|--------|----|----|
| Gateway | [en/gateway.md](docs/en/gateway.md) | [ru/gateway.md](docs/ru/gateway.md) |
| Adapters | [en/adapters.md](docs/en/adapters.md) | [ru/adapters.md](docs/ru/adapters.md) |
| Routing | [en/routing.md](docs/en/routing.md) | [ru/routing.md](docs/ru/routing.md) |
| Inspectors | [en/inspectors.md](docs/en/inspectors.md) | [ru/inspectors.md](docs/ru/inspectors.md) |
| Metering | [en/metering.md](docs/en/metering.md) | [ru/metering.md](docs/ru/metering.md) |
| Egress | [en/egress.md](docs/en/egress.md) | [ru/egress.md](docs/ru/egress.md) |
| Audit | [en/audit.md](docs/en/audit.md) | [ru/audit.md](docs/ru/audit.md) |
| Observability | [en/observability.md](docs/en/observability.md) | [ru/observability.md](docs/ru/observability.md) |
| Ingress Tunnel | [en/ingress.md](docs/en/ingress.md) | [ru/ingress.md](docs/ru/ingress.md) |
| Deployment | [en/deployment.md](docs/en/deployment.md) | [ru/deployment.md](docs/ru/deployment.md) |

Contracts: [`contracts/openapi.yaml`](contracts/openapi.yaml) · [`contracts/policies.schema.json`](contracts/policies.schema.json) · Config: [`py/fwllm/config.example.yaml`](py/fwllm/config.example.yaml)

## License & Commercial Use

Core: [FSL-1.1-MIT](LICENSE) — free except competing as LLM security gateway, auto-MIT after 2y.

**Open core:** gateway, routing, metering quotas, audit, egress direct/single_proxy, signatures, LightAnon DLP, metrics.

**Enterprise (commercial):** multi-pool proxies, ML models/update packs, UI, SIEM, RBAC/SSO, HA.

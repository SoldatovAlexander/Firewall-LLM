# Firewall LLM (fwllm)

Security gateway for LLM traffic: token spend control, data-leak prevention, prompt injection defense, shadow AI blocking.

![Python](https://img.shields.io/badge/python-3.11%2B-blue)
![Rust](https://img.shields.io/badge/rust-gateway-orange)
![License](https://img.shields.io/badge/license-FSL--1.1--MIT-blue)
![Deployment](https://img.shields.io/badge/deployment-on--prem-red)

## Project Description / Описание проекта

**EN:** Firewall LLM is an open-core security gateway that sits between applications and LLM providers. It exposes a single OpenAI-compatible API, routes requests to multiple models through swappable provider adapters, meters tokens and requests, masks sensitive data before it leaves the perimeter, detects prompt injection attacks, and switches providers automatically by policy — token budgets, request rates, attack detection, or provider health. Fully on-prem: no telemetry leaves the perimeter.

**RU:** Firewall LLM — open-core шлюз безопасности между приложениями и LLM-провайдерами. Единый OpenAI-совместимый API, маршрутизация на несколько моделей через сменные адаптеры провайдеров, учёт токенов и запросов, маскирование чувствительных данных до выхода за периметр, детекция prompt injection и автоматическое переключение провайдеров по политикам — бюджет токенов, частота запросов, детекция атак, состояние провайдера. Полностью on-prem: телеметрия не покидает контур.

## Features
- Unified OpenAI-compatible API (`POST /v1/chat/completions`, streaming SSE).
- One adapter per external LLM API: OpenRouter, OpenAI, Anthropic, Ollama/vLLM.
- Provider switching by policy: tokens spent, requests count, attack detection, error rate.
- Prompt injection detection: signature rules + local ONNX ML classifier, severity-based blocking.
- DLP masking via [LightAnon](https://github.com/SoldatovAlexander/lightanon_project): reversible sanitization of prompts, `ru_152` compliance profile.
- Egress control (open core): all adapters **direct** or through **one global proxy**.
- Enterprise: multi-pool egress proxies with rotation by request count (round_robin / random / least_used), failure thresholds and cooldowns, per-adapter bindings (`fwllm-enterprise` package).
- Prometheus metrics + dashboard import into an existing Grafana instance.
- Declarative YAML configuration (`config.example.yaml`), JSON Schema for policies.
- Full audit log with PII redaction (`/admin/audit`).
- Two branches, one contract: Python gateway + Rust production gateway.

## Architecture

```
Clients ──► Unified OpenAI-like API ──► CORE Gateway
                                          ├─ Router + Policy Engine
                                          ├─ Inspector (injection / DLP)
                                          ├─ Metering (tokens, quotas)
                                          ├─ Egress Proxy Pool
                                          ├─ Audit Log
                                          └─ /metrics ─► Prometheus ─► Grafana
                                                │
                        Provider Adapters: [openrouter] [openai] [anthropic] [ollama]
```

## Policy Example

```yaml
routing:
  state_store: redis          # memory | redis (survives restarts)
  default_chain: [openrouter, local-ollama]
  model_mapping:
    gpt-4o:
      openrouter: "meta-llama/llama-3.3-70b-instruct:free"
      local-ollama: "llama3.3:70b"
  rules:
    - name: token-budget-switch        # switch by tokens spent
      when:
        provider: openrouter
        provider_tokens_today: { gt: 5000000 }
      action:
        next_in_chain: true
  attack_failover:                     # switch on attack detection
    enabled: true
    count: 5
    window_seconds: 300
    min_severity: high
    switch_to: local-ollama
    block_source: true

inspectors:
  dlp:
    mode: mask            # block | mask | log | off
    restore_policy: mask
    profile: ru_152
  injection:
    mode: block
    block_severity_gte: high
    ml:
      enabled: true
      model_dir: /models/pi
      threshold: 0.30

egress:
  mode: direct            # direct | single_proxy | pools (enterprise)
```

## Installation & Quick Start (Python)

```bash
git clone https://github.com/SoldatovAlexander/Firewall-LLM.git
cd Firewall-LLM/py/fwllm
python3 -m venv .venv
source .venv/bin/activate
pip install -e '.[dev]'
cp config.example.yaml fwllm.yaml       # edit providers/quotas/policies
cp .env.example .env                    # OPENROUTER_API_KEY=..., FWLLM_CLIENT_TOKENS=key:label

FWLLM_CONFIG=./fwllm.yaml uvicorn fwllm.main:app --host 0.0.0.0 --port 8080
```

Test request:

```bash
curl http://127.0.0.1:8080/v1/chat/completions \
  -H "Authorization: Bearer <client-key>" \
  -H "Content-Type: application/json" \
  -d '{"model": "gpt-4o", "messages": [{"role": "user", "content": "Hello!"}]}'
```

The logical model (`gpt-4o`) is mapped to a concrete provider model via
`routing.model_mapping`; the router picks the active provider from the chain.
Endpoints: `/healthz`, `/v1/chat/completions`, `/metrics`, `/admin/audit`.

## Quick Start (Rust)

Production gateway in `rust/` (axum + tokio): same unified API, same contracts,
same metric names.

```bash
cd rust
cargo run --release -p fwllm-gateway   # FWLLM_CONFIG must point to fwllm.yaml
```

Implemented: bearer auth, non-streaming + SSE streaming completions,
model routing (chain / model_mapping / budget rules / attack failover),
metering quotas (Redis, fail-open), audit log (SQLite, PII redaction),
`/admin/audit`, Prometheus metrics.

## Deployment (on-prem)

Full stack: gateway + Redis + Prometheus + Grafana. Services auto-start on
reboot (Docker `restart: unless-stopped`).

```bash
cd deploy
cp fwllm.yaml.example fwllm.yaml      # edit providers/quotas/policies
cp .env.example .env                  # provider keys, client tokens, Grafana password
docker compose up -d --build
```

- Gateway: `http://<host>:8080`
- Import the dashboard into your Grafana:

```bash
python -m fwllm.observability.grafana_import \
  --url http://<grafana-host>:3000 --basic admin:<password> \
  --file grafana/dashboards/fwllm-overview.json \
  --datasource-url http://fwllm-prometheus:9090
```

Load test:

```bash
python py/fwllm/scripts/loadtest.py --url http://<host>:8080/v1/chat/completions \
  --key <client-key> --rps 50 --duration 20
```

Note: cloud LLM providers may block datacenter IPs ("Access denied by security
policy") — use `egress.mode: single_proxy` with an allowed egress proxy.

## Tests

TDD project: tests are written first for every module.

Python:

```bash
cd py/fwllm
pytest tests -q                    # unit + contract tests
pytest tests -q -m live            # live smoke against free OpenRouter models
ruff check src tests && mypy src
```

Rust:

```bash
cd rust
cargo test --workspace
cargo clippy --all-targets -- -D warnings
```

Training notebook for the ONNX injection classifier:
[`docs/notebooks/train_injection_model.ipynb`](docs/notebooks/train_injection_model.ipynb)
(Google Colab, free T4 is sufficient).

## Documentation
- Project brief: [`docs/BRIEF.md`](docs/BRIEF.md) ([.docx](docs/BRIEF.docx))
- Unified API contract: [`contracts/openapi.yaml`](contracts/openapi.yaml)
- Policies schema: [`contracts/policies.schema.json`](contracts/policies.schema.json)
- Example policy: [`contracts/policy.example.yaml`](contracts/policy.example.yaml)
- Example gateway config: [`py/fwllm/config.example.yaml`](py/fwllm/config.example.yaml)

## License & Commercial Use

Core is licensed under the [Functional Source License, Version 1.1, MIT Future
License](LICENSE) (FSL-1.1-MIT): you may use, study, modify and redistribute the
software for any purpose **except** offering a competing LLM security gateway
product. Two years after each version is published, it automatically becomes
MIT-licensed.

### What's included free (open core)
- Complete gateway: unified API, routing, metering quotas, audit log,
  egress direct/single_proxy, signature-based injection detection,
  LightAnon DLP masking, Prometheus metrics, dashboard import.

### Enterprise (commercial license)
- **Multi-pool egress proxies** — rotation by request count, healthchecks,
  per-adapter bindings (`fwllm-enterprise` package).
- **ML injection detector** — trained ONNX models and air-gapped update packs.
- Management UI/console, SIEM integrations (Splunk, Sentinel),
  granular RBAC/LDAP/SSO, HA clustering, centralized fleet management.
- Priority support and SLA for on-prem deployments.

Contact: repository owner for enterprise licensing terms.

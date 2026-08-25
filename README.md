# Firewall LLM (fwllm)

Security gateway for LLM traffic: token spend control, data-leak prevention, prompt injection defense, shadow AI blocking.

![Python](https://img.shields.io/badge/python-3.11%2B-blue)
![Rust](https://img.shields.io/badge/production-gateway-orange)
![License](https://img.shields.io/badge/license-TBD-lightgrey)
![Deployment](https://img.shields.io/badge/deployment-on--prem-red)

## Project Description / Описание проекта

**EN:** Firewall LLM is an open-core security gateway that sits between applications and LLM providers. It exposes a single OpenAI-compatible API, routes requests to multiple models through swappable provider adapters, meters tokens and requests, masks sensitive data before it leaves the perimeter, detects prompt injection attacks, and switches providers automatically by policy — token budgets, request rates, attack detection, or provider health. Fully on-prem: no telemetry leaves the perimeter.

**RU:** Firewall LLM — open-core шлюз безопасности между приложениями и LLM-провайдерами. Единый OpenAI-совместимый API, маршрутизация на несколько моделей через сменные адаптеры провайдеров, учёт токенов и запросов, маскирование чувствительных данных до выхода за периметр, детекция prompt injection и автоматическое переключение провайдеров по политикам — бюджет токенов, частота запросов, детекция атак, состояние провайдера. Полностью on-prem: телеметрия не покидает контур.

## Features
- Unified OpenAI-compatible API (`POST /v1/chat/completions`, streaming SSE).
- One adapter per external LLM API: OpenRouter, OpenAI, Anthropic, Ollama/vLLM.
- Provider switching by policy: tokens spent, requests count, attack detection, error rate.
- DLP masking via [LightAnon](https://github.com/SoldatovAlexander/lightanon_project): reversible sanitization of prompts, `ru_152` compliance profile.
- Prompt injection detection with severity-based blocking.
- Egress proxy pools bound to adapters, rotation by configurable request count.
- Prometheus metrics + dashboard import into an existing Grafana instance.
- Declarative YAML policies (`contracts/policies.schema.json`), hot reload.
- Full audit log with PII redaction.
- Two branches, one contract: Python MVP + Rust production gateway.

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
  default_chain: [openrouter, local-ollama]

rules:
  - name: token-budget-switch          # switch by tokens spent
    when:
      provider: openrouter
      provider_tokens_today: { gt: 5000000 }
    action:
      switch_to: local-ollama

  - name: rate-switch                  # switch by request count
    when:
      requests_per_minute: { gt: 500 }
    action:
      switch_to: local-ollama

  - name: attack-failover              # switch on attack detection
    when:
      event: attack_detected
      severity: { gte: high }
      count: { gt: 3 }
      window: 5m
    action:
      block_source: true
      switch_default_to: local-ollama
```

## Installation

```bash
git clone https://github.com/SoldatovAlexander/Firewall-LLM.git
cd Firewall-LLM/py/fwllm
python3 -m venv .venv
source .venv/bin/activate
pip install -e '.[dev]'
cp ../../contracts/policy.example.yaml ./policy.yaml   # edit to taste
export OPENROUTER_API_KEY="sk-or-v1-..."
```

## Quick Start

```bash
uvicorn fwllm.app:app --host 127.0.0.1 --port 8080
```

```bash
curl http://127.0.0.1:8080/v1/chat/completions \
  -H "Authorization: Bearer <client-key>" \
  -H "Content-Type: application/json" \
  -d '{
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "Hello!"}]
      }'
```

The logical model (`gpt-4o`) is mapped to a concrete provider model via the policy's
`model_mapping`; the router picks the active provider from the chain.

## Tests

TDD project: tests are written first for every module.

```bash
pytest tests -q                    # unit + contract tests
pytest tests -q -m live            # live smoke against free OpenRouter models
ruff check src tests               # lint
mypy src                           # type check
```

## Documentation
- Implementation plan (TDD phases): [`docs/PLAN.md`](docs/PLAN.md)
- Project brief: [`docs/BRIEF.md`](docs/BRIEF.md) ([.docx](docs/BRIEF.docx))
- Unified API contract: [`contracts/openapi.yaml`](contracts/openapi.yaml)
- Policies schema: [`contracts/policies.schema.json`](contracts/policies.schema.json)
- Example policy: [`contracts/policy.example.yaml`](contracts/policy.example.yaml)

## Roadmap

| Milestone | Scope | Status |
|---|---|---|
| M1 | Proxy to free models (gateway + adapters) | in progress |
| M2 | Token metering + Grafana dashboard | planned |
| M3 | Security: DLP/injection + egress proxy pool | planned |
| M4 | Auto provider switching + audit log — MVP complete | planned |

## License

Open-core: gateway core will be released under a source-available license
(FSL/BSL or AGPL — TBD before first public release). Enterprise modules
(ML detectors, UI, SIEM integrations, HA) are commercial.

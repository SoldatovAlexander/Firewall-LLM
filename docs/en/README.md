# Firewall LLM — Documentation (EN)

- [Gateway](gateway.md) — unified OpenAI-compatible API, auth, errors, streaming
- [Adapters](adapters.md) — provider registry, OpenRouter / Ollama / Tunnel
- [Routing](routing.md) — chains, model mapping, budget rules, attack failover
- [Inspectors](inspectors.md) — DLP (LightAnon) + injection (signatures + ML)
- [Metering](metering.md) — counters, quotas, fail-open
- [Egress](egress.md) — direct / single_proxy / pools (enterprise) / tunnel
- [Audit](audit.md) — SQLite, redaction, `/admin/audit`
- [Observability](observability.md) — Prometheus `/metrics`, Grafana import
- [Ingress Tunnel](ingress.md) — `wss://:8443`, tokens, agent, masking
- [Deployment](deployment.md) — Docker Compose, Helm, certs

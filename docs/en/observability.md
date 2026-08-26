# Observability

`GET /metrics` — Prometheus:
* `fw_requests_total{client,provider,model,code}`
* `fw_tokens_total{client,provider,model,direction}`
* `fw_request_duration_seconds{provider,model}`

Import: `python -m fwllm.observability.grafana_import --url http://grafana:3000 --basic admin:pass --file grafana/dashboards/fwllm-overview.json`

# Firewall-LLM Telemetry Prototype

This stack proves one distributed trace across a Python agent and the real Rust Firewall-LLM gateway.

## Start

From this directory:

```bash
docker compose up --build
```

The `demo-agent` waits for the gateway, performs two non-streaming LLM calls inside one `invoke_agent` trace, prints the trace ID, and exits.

Open Jaeger at:

```text
http://localhost:16686
```

Search for service `fwllm-demo-agent` or `fwllm-gateway` and use the printed trace ID.

Expected structure:

```text
invoke_agent
├── retrieval
├── fwllm.request
│   └── fwllm.provider.request
├── execute_tool
└── fwllm.request
    └── fwllm.provider.request
```

## Re-run only the agent

```bash
docker compose run --rm demo-agent
```

## Fail-open preparation

Collector outage is a later formal experiment. The gateway exporter is asynchronous; the request path must not synchronously depend on Collector availability.

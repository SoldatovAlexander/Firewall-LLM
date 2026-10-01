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


## Collector outage / fail-open smoke test

The prototype includes a reproducible smoke test that stops the OpenTelemetry
Collector and then performs a real chat request through the Rust gateway.

```bash
chmod +x check-fail-open.sh
./check-fail-open.sh
```

Expected result:

```text
PASS: gateway request path remains available with Collector stopped.
```

To keep the stack running after the test:

```bash
KEEP_STACK=1 ./check-fail-open.sh
```

This smoke test proves the availability property at deployment level. Formal
experiment **E4** (including evidence, exporter errors and timing) is recorded
in PR-5.


## Formal E1–E7 experiment suite

Run the complete architecture validation:

```bash
chmod +x run-experiments.sh
./run-experiments.sh
```

Evidence is written to:

```text
artifacts/
├── normal-trace.json
├── security-trace.json
├── e1-e2-e6.json
├── e3-security.json
├── e4-fail-open.json
├── e5-load.json
├── e5-docker-stats-before.json
├── e5-docker-stats-after.json
├── e7-enabled.json
├── e7-disabled.json
├── e7-performance.json
└── EXPERIMENT_SUMMARY.md
```

Methodology and decision rules are documented in
`docs/PROTOTYPE_OTEL_EXPERIMENTS.md`.

# Firewall-LLM Telemetry Demo Agent

A deliberately small framework-neutral agent used by the architecture prototype.

It creates the trace:

```text
invoke_agent
├── retrieval
├── fwllm.request
│   └── fwllm.provider.request
├── execute_tool
└── fwllm.request
    └── fwllm.provider.request
```

The demo exports its own spans to the same OpenTelemetry Collector as the Rust gateway and propagates W3C `traceparent` on every Firewall-LLM request.

No prompt or response content is attached to telemetry attributes.

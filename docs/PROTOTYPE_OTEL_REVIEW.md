# Firewall-LLM OpenTelemetry Prototype Review

**Date:** 2026-10-02  
**Decision:** **GO → Pilot Hardening**  
**Production readiness:** No  
**Evidence workflow:** GitHub Actions run `36930377923`  
**Evidence artifact:** `fwllm-otel-prototype-evidence` / `11194979948`

## Executive conclusion

OpenTelemetry architecture is technically viable for Firewall-LLM as a **Secure Agent Observability Gateway**.

The prototype demonstrated that Firewall-LLM can join agent-side and gateway-side execution into one distributed trace, attach provider metadata, explain a security block in that trace, and keep the request path available when the telemetry Collector is unavailable.

The prototype therefore passes the architecture gate.

The next stage is **Pilot Hardening**, not production release.

## Evidence summary

| Experiment | Status | Evidence |
|---|---|---|
| E1 Distributed trace | PASS | Agent and Rust gateway appear in one W3C trace |
| E2 Provider correlation | PASS | Two provider spans with provider/model metadata |
| E3 Security explainability | PASS | Injection block visible; provider is not invoked |
| E4 Collector fail-open | PASS | Collector stopped; request HTTP 200; gateway healthy |
| E5 Queue/load resilience | PARTIAL | 1200/1200 successes at concurrency 8; internal queue/drop boundedness unmeasured |
| E6 Privacy metadata-only | PASS | Unique prompt sentinel absent from exported trace |
| E7 Performance baseline | PASS | ON/OFF p50/p95/p99 measured reproducibly |

## E4 / E5 resilience evidence

Collector was stopped before E4/E5.

E4:

```text
HTTP status       200
gateway health    true
latency           22.366 ms
choices payload   present
```

E5:

```text
requests          1200
concurrency       8
successes         1200
gateway health    true
elapsed           1.987 s
throughput        604.023 req/s
p50               12.475 ms
p95               16.097 ms
p99               18.475 ms
```

This is strong evidence that Collector loss does not synchronously break the gateway.

It is not sufficient evidence that exporter queue/memory behavior is bounded over long outages.

## E7 performance evidence

Telemetry enabled:

```text
mean              3.840 ms
p50               3.784 ms
p95               3.997 ms
p99               4.154 ms
```

Telemetry disabled:

```text
mean              3.927 ms
p50               3.859 ms
p95               4.246 ms
p99               4.822 ms
```

The enabled run being slightly faster is measurement noise, not a telemetry speedup.

Decision:

> no measurable latency penalty was detected in this short synthetic baseline.

A pilot acceptance threshold must be set after multi-run and realistic-workload measurements.

## Confirmed architectural decisions

1. Keep OpenTelemetry/OTLP as the telemetry standard.
2. Keep observability storage external to Firewall-LLM.
3. Preserve W3C trace context as the correlation mechanism.
4. Preserve domain-first security events; OTel stays an adapter/consumer.
5. Preserve metadata-only as default.
6. Keep telemetry asynchronous/fail-open.
7. Keep existing Prometheus metrics; traces complement rather than replace them.
8. Continue Python agent instrumentation + Rust gateway instrumentation.

## Hardening gaps before pilot

### H1 — Telemetry pipeline health

Required:

- exporter errors;
- dropped spans;
- queue pressure/size where technically available;
- export latency;
- explicit telemetry health status.

Purpose: close E5 from PARTIAL to PASS or define an explicit accepted limitation.

### H2 — Long Collector outage

Repeat with:

- longer outage;
- sustained request volume;
- memory high-water measurement;
- recovery after Collector returns.

### H3 — Performance methodology

Repeat E7 with:

- multiple runs;
- larger sample;
- concurrent traffic;
- realistic provider latency;
- streaming traffic;
- p50/p95/p99 and CPU/memory.

### H4 — Streaming formal evidence

PR-4 code is regression-tested, but Pilot Hardening should capture an actual Jaeger streaming trace for normal completion, upstream error and client disconnect.

### H5 — Privacy/security expansion

Add controlled tests for:

- DLP block;
- DLP mask/redaction;
- secret filtering;
- safe exception/error telemetry;
- external provider boundary.

## Pilot Hardening exit gate

Proceed to customer pilot when:

- H1 has measurable telemetry health;
- Collector outage recovery is tested;
- performance baseline is repeated on realistic traffic;
- streaming trace lifecycle has evidence;
- privacy/security tests cover DLP + injection;
- integration quickstart remains low-friction.

## Decision rationale

The architecture does not show a blocking flaw.

The only incomplete prototype experiment is E5 boundedness, and the observed behavior under Collector outage is positive: 1200/1200 successful requests and a healthy gateway.

Therefore the appropriate decision is:

> **GO to Pilot Hardening with E5 telemetry-health work as a mandatory condition before pilot sign-off.**

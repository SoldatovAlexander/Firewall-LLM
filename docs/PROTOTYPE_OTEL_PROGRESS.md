# Firewall-LLM OpenTelemetry Prototype — Progress & Decision Log

**Статус:** active prototype  
**Дата фиксации:** 2026-10-02  
**Цель prototype-track:** проверить архитектурную гипотезу Firewall-LLM как Secure Agent Observability Gateway и основу AI Control Plane без построения собственного observability backend.

## 1. Целевая гипотеза

Firewall-LLM должен давать сквозную видимость цепочки:

```text
User / Agent
   ↓
Agent runtime
   ↓
Firewall-LLM
   ↓
Security / Policy / Routing
   ↓
LLM Provider / Private LLM
   ↓
Tool / MCP / Retrieval
```

Ключевой принцип:

> Observe → Explain → Protect → Control → Govern → Optimize.

На prototype-этапе реализуются первые три шага: **Observe → Explain → Protect**.

## 2. Зафиксированные архитектурные решения

1. **OpenTelemetry / OTLP** используется как стандарт телеметрии.
2. Firewall-LLM **не строит собственное trace/log/metric хранилище**.
3. Полный trace требует двух уровней instrumentation:
   - agent-side spans;
   - gateway/security spans Firewall-LLM.
4. Контекст связывается через W3C `traceparent` / `tracestate`.
5. Telemetry pipeline не должен становиться зависимостью request path.
6. Privacy-by-default:
   - content capture выключен;
   - metadata-only по умолчанию;
   - prompts / completions / secrets не попадают в telemetry attributes.
7. Доменные модули не должны зависеть от OpenTelemetry API.
8. Security decisions должны быть объяснимы внутри того же distributed trace.
9. Python/Rust должны сохранять общий telemetry contract.
10. Текущие Prometheus metrics сохраняются и не заменяются OpenTelemetry trace pipeline.

## 3. Реализовано

### PR-1 — OpenTelemetry Foundation

Branch:

```text
feature/otel-prototype
```

Pull Request:

```text
#1 Prototype PR-1: OpenTelemetry foundation
```

Состав:

- telemetry config в `fwllm-core`;
- telemetry disabled by default;
- OpenTelemetry SDK / OTLP gRPC exporter;
- `tracing-opentelemetry`;
- W3C TraceContext propagator;
- `metadata_only` content mode;
- валидация telemetry config;
- обновлённый `Cargo.lock`;
- lifecycle `TelemetryRuntime`;
- existing `fmt + RUST_LOG` сохранён.

Проверки:

```text
cargo clippy ... -- -D warnings      PASS
cargo test --workspace --locked      PASS
```

### PR-2 — First End-to-End Trace

Branch:

```text
feature/otel-first-trace
```

Pull Request:

```text
#2 Prototype PR-2: First end-to-end trace
```

Состав:

- inbound W3C `traceparent` extraction;
- `fwllm.request` server span;
- request metadata:
  - request id;
  - requested model;
  - stream flag;
- `fwllm.provider.request` child span для non-streaming вызова;
- framework-neutral Python Demo Agent;
- agent spans:
  - `invoke_agent`;
  - `retrieval`;
  - `execute_tool`;
- isolated prototype deployment:
  - Demo Agent;
  - Rust Firewall-LLM;
  - Mock LLM;
  - Redis;
  - OpenTelemetry Collector;
  - Jaeger.

Ожидаемый trace:

```text
invoke_agent
├── retrieval
├── fwllm.request
│   └── fwllm.provider.request
├── execute_tool
└── fwllm.request
    └── fwllm.provider.request
```

Проверки:

```text
Rust clippy       PASS
Rust tests        PASS
Python lint       PASS
Python mypy       PASS
Python tests      PASS
```

## 4. Что сознательно НЕ реализовано в PR-1 / PR-2

- SecurityFinding typed domain event;
- `fwllm.security.inspect`;
- `prompt_injection.detected`;
- DLP telemetry events;
- runtime policy telemetry;
- streaming provider span lifecycle;
- telemetry drop / queue health metrics;
- formal Collector outage experiment;
- performance overhead experiment;
- trace sampling policy;
- failover span;
- cost attribution;
- semantic policy engine;
- Agent Registry / Tool Registry;
- Human-in-the-loop;
- Shadow AI discovery;
- Kill Switch / Quarantine.

## 5. PR-3 — Security & Sanitization

Цель:

> превратить distributed trace из обычного observability trace в объяснимый security trace Firewall-LLM.

План:

1. Ввести typed domain model `SecurityFinding`.
2. Убрать callback с неструктурированными `String` параметрами.
3. Использовать composite sink:
   ```text
   SecurityFinding
      ├── Router reaction
      └── Telemetry event
   ```
4. Добавить span `fwllm.security.inspect`.
5. Добавить event `prompt_injection.detected`.
6. Зафиксировать security attributes:
   - category;
   - rule;
   - severity;
   - action.
7. Ввести explicit allowlist telemetry attributes.
8. Не экспортировать raw prompt / completion / secret / provider body.
9. Добавить safe provider error mapping.
10. E2E security test:
    ```text
    malicious prompt
        ↓
    injection detector
        ↓
    SecurityFinding
        ↓
    block
        ↓
    same distributed trace
        ↓
    provider NOT called
    ```

### Acceptance criteria PR-3

- security finding является domain object, а не OTel-specific struct;
- router продолжает получать attack signal;
- blocked request не вызывает provider;
- telemetry содержит rule/severity/action;
- telemetry не содержит raw prompt;
- блокировка видна в том же trace;
- existing security tests не регрессируют;
- Rust CI зелёный.

## 6. PR-4 — Streaming + Fail-open Telemetry

План:

- lifecycle span для SSE/streaming;
- корректное закрытие span при:
  - terminal chunk;
  - upstream error;
  - client disconnect;
  - timeout;
- bounded telemetry behavior;
- Collector outage experiment;
- telemetry exporter failure не влияет на gateway availability;
- telemetry health counters.

## 7. PR-5 — Prototype Experiments E1–E7

Формальный набор архитектурных экспериментов:

| ID | Эксперимент | Проверяем |
|---|---|---|
| E1 | Agent → Gateway distributed trace | W3C context continuity |
| E2 | Provider trace | latency/provider/model correlation |
| E3 | Security block trace | explainable security decision |
| E4 | Collector unavailable | fail-open request path |
| E5 | Telemetry queue/load | bounded resource behavior |
| E6 | Privacy validation | no raw PII/secrets export |
| E7 | Performance baseline | p50/p95/p99 overhead |

Результат каждого эксперимента фиксируется как:

```text
PASS / PARTIAL / FAIL
Evidence
Observed limitation
Decision
Follow-up
```

## 8. Prototype Review Gate

После PR-5 проводится решение:

### GO

Если:

- distributed trace стабилен;
- request path не зависит от Collector;
- security decisions объяснимы;
- privacy-by-default подтверждена;
- integration friction приемлем;
- overhead измерен и приемлем для pilot.

### REWORK

Если основная архитектура подтверждена, но остаются исправимые проблемы:

- потеря trace context;
- чрезмерный telemetry overhead;
- неудобная agent instrumentation;
- высокий drop rate;
- недостаточная security semantics.

### NO-GO

Если:

- OTel pipeline требует опасной связанности с request path;
- telemetry нельзя сделать privacy-safe;
- security events невозможно надежно связать с trace;
- integration overhead делает решение практически неприменимым.

## 9. Текущая веточная схема

```text
main
 │
 └── PR-1 / feature/otel-prototype
       │
       └── PR-2 / feature/otel-first-trace
             │
             └── PR-3 / feature/otel-security
                   │
                   └── PR-4 / feature/otel-streaming
                         │
                         └── PR-5 / feature/otel-experiments
```

Все prototype PR до Prototype Review остаются draft и не обязаны сразу попадать в `main`.

## 10. Product meaning

PR-1 и PR-2 подтверждают техническую основу:

> Firewall-LLM способен связать agent-side execution и gateway/provider execution в один distributed trace.

PR-3 должен подтвердить ключевой differentiator:

> Firewall-LLM способен не только показать, что агент вызвал LLM, но и объяснить, какое security-событие произошло, какое решение было принято и почему выполнение было заблокировано — в рамках того же trace.

Это и есть переход от обычного observability gateway к **Secure Agent Observability Gateway**.

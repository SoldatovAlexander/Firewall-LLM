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

## 5. PR-3 — Security & Sanitization — IMPLEMENTED

**Branch:** `feature/otel-security`  
**Pull Request:** `#3 Prototype PR-3: Security findings and trace semantics`  
**CI:** Rust clippy PASS, workspace tests PASS.

Цель:

> превратить distributed trace из обычного observability trace в объяснимый security trace Firewall-LLM.

Реализовано:

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

### Acceptance criteria PR-3 — результат

- security finding является domain object, а не OTel-specific struct — **PASS**;
- router продолжает получать attack signal через composite sink — **PASS**;
- blocked request не вызывает provider — **PASS**;
- telemetry allowlist содержит category/rule/severity/action — **PASS**;
- client ID, raw prompt, completion и secrets не входят в security telemetry attributes — **PASS**;
- security inspection выполняется внутри `fwllm.security.inspect` — **PASS**;
- provider telemetry содержит только безопасные error type/status, без provider body — **PASS**;
- existing security tests не регрессируют — **PASS**;
- Rust clippy/test CI — **PASS**.

### Фактическая схема PR-3

```text
Security detector
      ↓
SecurityFinding (domain)
      ├── Router reaction
      └── Telemetry adapter
             ↓
       fwllm.security.inspect
             ↓
       prompt_injection.detected
             ↓
         action=block
```

При блокировке `fwllm.provider.request` не создаётся, потому что provider не вызывается.

## 6. PR-4 — Streaming + Fail-open Telemetry — IMPLEMENTED

**Branch:** `feature/otel-streaming`  
**Pull Request:** `#4 Prototype PR-4: Streaming telemetry and fail-open smoke test`  
**CI:** Rust clippy PASS, workspace tests PASS, Python CI PASS.

Реализовано:

- `fwllm.provider.request` для streaming/SSE;
- span создаётся как child текущего `fwllm.request`;
- span живёт дольше handler future и удерживается body stream;
- exact-once telemetry finalization отделён от metering/audit atomic state;
- terminal states:
  - `ok` — нормальный `[DONE]`;
  - `error` — stream/open/provider error;
  - `cancelled` — client disconnect / body Drop;
- stream-open HTTP error экспортирует только safe type/status;
- provider body/error text не экспортируется в telemetry attributes;
- существующий StreamAccountant / RequestLifecycle сохранён;
- добавлен `deploy/telemetry-prototype/check-fail-open.sh`;
- smoke-test останавливает Collector и проверяет `/healthz` + реальный chat request.

### Результат PR-4

- streaming lifecycle компилируется и проходит existing streaming regression suite — **PASS**;
- Python/deploy regression — **PASS**;
- strict clippy — **PASS**;
- fail-open сценарий воспроизводим — **READY FOR E4**;
- формальный Collector outage result — **PENDING PR-5**;
- telemetry queue/drop health instrumentation — **PENDING E5 / post-prototype**, если не потребуется для архитектурного решения.

## 7. PR-5 — Prototype Experiments E1–E7 — EXECUTED

**Branch:** `feature/otel-experiments`  
**Pull Request:** `#5 Prototype PR-5: Formal OpenTelemetry experiments E1-E7`  
**Workflow run:** `36930377923` — SUCCESS  
**Evidence artifact:** `fwllm-otel-prototype-evidence`, artifact ID `11194979948`  
**Artifact SHA-256:** `3cbe4d6496fbd5bf2cf7830535d06b9b2004600fa68a4584a343fc365d52c15a`

Фактические результаты:

| ID | Результат | Ключевое evidence |
|---|---|---|
| E1 | **PASS** | один trace содержит `fwllm-demo-agent` + `fwllm-gateway`, agent/retrieval/tool/request spans |
| E2 | **PASS** | 2 provider spans; provider=`mock`, model=`prototype-model` |
| E3 | **PASS** | security trace содержит `fwllm.security.inspect`, rule/severity/action; provider span отсутствует |
| E4 | **PASS** | Collector stopped → HTTP 200, health=true, latency 22.366 ms |
| E5 | **PARTIAL** | 1200/1200 успешных запросов, concurrency 8, gateway healthy; queue/drop boundedness не измерена |
| E6 | **PASS** | synthetic privacy sentinel отсутствует в Jaeger JSON |
| E7 | **PASS** | telemetry ON/OFF baseline воспроизводимо измерен |

### E5 фактические числа

```text
requests        1200
concurrency     8
successes       1200
gateway health  true
elapsed         1.987 s
throughput      604.023 req/s
p50             12.475 ms
p95             16.097 ms
p99             18.475 ms
```

Статус остаётся **PARTIAL**, потому что survival под нагрузкой не доказывает bounded exporter queue / dropped spans / memory pressure.

### E7 фактические числа

Telemetry enabled:

```text
mean  3.840 ms
p50   3.784 ms
p95   3.997 ms
p99   4.154 ms
```

Telemetry disabled:

```text
mean  3.927 ms
p50   3.859 ms
p95   4.246 ms
p99   4.822 ms
```

Полученные отрицательные delta не трактуются как «telemetry ускоряет gateway». Разница находится в области шума короткого synthetic benchmark. Корректный вывод: **в этом prototype-run измеримого latency regression не обнаружено**; production/pilot threshold должен определяться повторными измерениями на реалистичной нагрузке.

Полная методика: `docs/PROTOTYPE_OTEL_EXPERIMENTS.md`.

## 8. Prototype Review Gate — DECISION RECORDED

### Решение: **GO → Pilot Hardening**

Prototype подтвердил основную архитектурную гипотезу:

- distributed trace между agent и gateway работает;
- provider execution коррелируется;
- security decision объясним в том же trace;
- blocked security request не достигает provider;
- Collector не является synchronous dependency request path;
- metadata-only не экспортировал synthetic sensitive sentinel;
- synthetic performance baseline не показал измеримого regression.

Это **не** означает production-ready. Перед pilot необходимо закрыть E5 hardening gap и повторить performance/resilience tests на более реалистичной нагрузке.

После PR-5 критерии решения интерпретируются так:

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

PR-3 и формальный E3 подтвердили ключевой differentiator:

> Firewall-LLM способен не только показать, что агент вызвал LLM, но и объяснить, какое security-событие произошло, какое решение было принято и почему выполнение было заблокировано — в рамках того же trace.

PR-5 дополнительно подтвердил fail-open request path и metadata-only privacy property.

Итог prototype-track:

> **Архитектура Secure Agent Observability Gateway подтверждена для перехода в Pilot Hardening.**

# Benchmark: Python vs Rust Gateway (dev-server 192.168.88.101)

> **Архивный замер (релиз 0.1.0):** цифры ниже от 2026-08-26 — до цикла R01–R16. Учёт токенов/квот с тех пор изменился (R03 always-count + estimate, R05 атомарный admit/settle с резервом), поэтому абсолютные значения RPS/памяти нельзя переносить на релиз 1:1. Перегон запланирован на dev-server после обновления стенда.

**Date:** 2026-08-26 · **Tool:** `scripts/bench.py` (httpx, asyncio, 50 rps, 10s, 3 runs) · **Provider:** mock-llm (http://mock-llm:8000, instant 200)

## Release 0.1.0 smoke (2026-09-06, dev-server, no-AVX2 CPU)

Prod-like config: DLP `mask/restore` `ru_152`, injection `block` (+ML XLM-R on Python; **Rust ML off — this host lacks AVX2 required by ort prebuilt binaries**, signatures only), mock upstream, shared Redis. `scripts/bench.py`.

| Load | Python chat p50/p95 | Rust chat p50/p95 | healthz p50 |
|---|---|---|---|
| 5 rps × 15 s | 60 / 88 ms | 23 / 75 ms | ~7 ms both |
| 20 rps × 10 s | 789 / 1371 ms (saturating) | 21 / 29 ms (flat) | ~8 ms both |
| 50 rps × 10 s ×3 runs | 2324 / 2570 ms med (2070–2683 / 2522–3367) | 18 / 22 ms med (flat) | ~6–10 ms both |

Codes at every load: all `200` (Python included) — degradation is latency-only, no errors. Reading: on this hardware Python chat saturates between 5 and 20 rps (ONNX inference without AVX2 dominates); Rust stays flat to at least 50 rps. Capacity planning must use these numbers, not the 2026-08-26 table (DLP-off, ML-off, pre-R03/R05 accounting).

Hardware constraints affecting the release image: the `gateway-rust` image requires trixie glibc (ort prebuilts need ≥ 2.38) + libstdc++/libgomp at runtime (see Dockerfile); ort prebuilts additionally require AVX2 — hosts without it must set `injection.ml.enabled: false` on the Rust gateway (fail-fast aborts startup otherwise).

## Healthz (no provider, pure gateway)

| Gateway | RPS | p50 | p95 | Memory |
|---------|-----|-----|-----|--------|
| Python (FastAPI) | 47.6 | 7 ms | 10 ms | 2.6 GiB |
| Rust (axum)      | 47.4 | 6 ms | 9 ms  | 1.4 MiB |

## Chat Completions (mock provider, DLP off for bench)

| Gateway | RPS | p50 | p95 | Memory | Notes |
|---------|-----|-----|-----|--------|-------|
| Python | 48.3 | 15 ms | 17 ms | 2.6 GiB | LightAnon + onnxruntime loaded (1.1 GiB model) |
| Rust   | 47.6 | 10 ms | 13 ms | 1.4 MiB | regex DLP, no ML model in bench config |

**Error path (openrouter blocked, DLP on, real provider):**

| Gateway | p50 | p95 | Result |
|---------|-----|-----|--------|
| Python | 1960 ms | 4020 ms | 502 upstream_error (IP blocked) |
| Rust   | 25 ms   | 45 ms   | 502 upstream_error |

Rust is **~5–80× faster** on the error path and **~1500× more memory efficient** (3 MiB vs 3 GiB) when the ML model is not loaded; with the 1.1 GiB ONNX model Python is ~2.6 GiB vs Rust ~200 MiB (when ML is enabled, not measured in this bench).

Run:
```bash
py/fwllm/.venv/bin/python scripts/bench.py --key bench-key --rps 50 --duration 10
# healthz + chat via mock-llm when fwllm-bench.yaml is active:
# docker compose -f docker-compose.yml -f docker-compose.bench.yml up -d gateway gateway-rust
```

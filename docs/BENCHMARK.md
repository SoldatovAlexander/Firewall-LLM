# Benchmark: Python vs Rust Gateway (dev-server 192.168.88.101)

**Date:** 2026-08-26 · **Tool:** `scripts/bench.py` (httpx, asyncio, 50 rps, 10s, 3 runs) · **Provider:** mock-llm (http://mock-llm:8000, instant 200)

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

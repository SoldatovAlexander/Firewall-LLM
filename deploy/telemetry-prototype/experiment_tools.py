#!/usr/bin/env python3
"""Executable evidence helpers for the Firewall-LLM OTel prototype."""

from __future__ import annotations

import argparse
import json
import os
import statistics
import sys
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from typing import Any

DEFAULT_GATEWAY = "http://localhost:8081"
DEFAULT_JAEGER = "http://localhost:16686"
TOKEN = "prototype-token"


def fetch_json(url: str, timeout: float = 5.0) -> Any:
    with urllib.request.urlopen(url, timeout=timeout) as response:
        return json.loads(response.read().decode("utf-8"))


def wait_url(url: str, timeout: float = 90.0) -> None:
    deadline = time.monotonic() + timeout
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            with urllib.request.urlopen(url, timeout=2) as response:
                if 200 <= response.status < 300:
                    print(f"READY {url}")
                    return
        except Exception as exc:  # pragma: no cover - diagnostic loop
            last_error = exc
        time.sleep(1)
    raise RuntimeError(f"timeout waiting for {url}: {last_error}")


def trace_operations(payload: dict[str, Any]) -> list[str]:
    data = payload.get("data") or []
    if not data:
        return []
    return [
        span.get("operationName", "")
        for span in data[0].get("spans", [])
    ]


def trace_services(payload: dict[str, Any]) -> set[str]:
    data = payload.get("data") or []
    if not data:
        return set()
    processes = data[0].get("processes", {})
    return {
        process.get("serviceName", "")
        for process in processes.values()
        if process.get("serviceName")
    }


def span_tags(span: dict[str, Any]) -> dict[str, Any]:
    return {
        tag.get("key"): tag.get("value")
        for tag in span.get("tags", [])
        if tag.get("key")
    }


def wait_trace(
    jaeger: str,
    trace_id: str,
    required: list[str],
    output: Path,
    timeout: float = 45.0,
) -> None:
    deadline = time.monotonic() + timeout
    last_ops: list[str] = []
    url = f"{jaeger.rstrip('/')}/api/traces/{trace_id}"

    while time.monotonic() < deadline:
        try:
            payload = fetch_json(url)
            last_ops = trace_operations(payload)
            if payload.get("data") and all(op in last_ops for op in required):
                output.parent.mkdir(parents=True, exist_ok=True)
                output.write_text(
                    json.dumps(payload, indent=2, sort_keys=True),
                    encoding="utf-8",
                )
                print(
                    json.dumps(
                        {
                            "trace_id": trace_id,
                            "operations": sorted(set(last_ops)),
                            "output": str(output),
                        },
                        indent=2,
                    )
                )
                return
        except Exception:
            pass
        time.sleep(1)

    raise RuntimeError(
        f"trace {trace_id} did not contain required operations "
        f"{required}; observed={sorted(set(last_ops))}"
    )


def validate_normal(trace_file: Path, sentinel: str) -> dict[str, Any]:
    raw = trace_file.read_text(encoding="utf-8")
    payload = json.loads(raw)
    ops = trace_operations(payload)
    services = trace_services(payload)

    e1_required = {
        "invoke_agent",
        "retrieval",
        "execute_tool",
        "fwllm.request",
    }
    e1_missing = sorted(e1_required.difference(ops))
    e1_services = {
        "fwllm-demo-agent",
        "fwllm-gateway",
    }
    missing_services = sorted(e1_services.difference(services))

    provider_spans = [
        span
        for span in payload["data"][0].get("spans", [])
        if span.get("operationName") == "fwllm.provider.request"
    ]
    provider_tags = [span_tags(span) for span in provider_spans]
    e2_ok = bool(provider_spans) and all(
        tags.get("gen_ai.provider.name")
        and tags.get("gen_ai.request.model")
        for tags in provider_tags
    )

    e6_ok = bool(sentinel) and sentinel not in raw

    result = {
        "E1": {
            "status": "PASS" if not e1_missing and not missing_services else "FAIL",
            "evidence": {
                "missing_operations": e1_missing,
                "services": sorted(services),
                "missing_services": missing_services,
            },
        },
        "E2": {
            "status": "PASS" if e2_ok else "FAIL",
            "evidence": {
                "provider_span_count": len(provider_spans),
                "provider_tags": provider_tags,
            },
        },
        "E6": {
            "status": "PASS" if e6_ok else "FAIL",
            "evidence": {
                "sentinel_present_in_trace": sentinel in raw,
                "sentinel_length": len(sentinel),
            },
        },
    }

    if any(entry["status"] == "FAIL" for entry in result.values()):
        raise RuntimeError(json.dumps(result, indent=2))
    return result


def validate_security(trace_file: Path) -> dict[str, Any]:
    raw = trace_file.read_text(encoding="utf-8")
    payload = json.loads(raw)
    ops = trace_operations(payload)

    required_ops = {"invoke_agent", "fwllm.request", "fwllm.security.inspect"}
    missing = sorted(required_ops.difference(ops))
    required_values = [
        "prompt_injection.detected",
        "override_instructions",
        "critical",
        "block",
    ]
    missing_values = [value for value in required_values if value not in raw]
    provider_called = "fwllm.provider.request" in ops

    status = (
        "PASS"
        if not missing and not missing_values and not provider_called
        else "FAIL"
    )
    result = {
        "E3": {
            "status": status,
            "evidence": {
                "missing_operations": missing,
                "missing_security_values": missing_values,
                "provider_span_present": provider_called,
                "operations": sorted(set(ops)),
            },
        }
    }
    if status == "FAIL":
        raise RuntimeError(json.dumps(result, indent=2))
    return result


def post_chat(
    gateway: str,
    text: str = "prototype experiment request",
    timeout: float = 10.0,
) -> tuple[int, float, dict[str, Any] | str]:
    payload = json.dumps(
        {
            "model": "prototype-model",
            "messages": [{"role": "user", "content": text}],
            "stream": False,
        }
    ).encode("utf-8")
    request = urllib.request.Request(
        f"{gateway.rstrip('/')}/v1/chat/completions",
        data=payload,
        method="POST",
        headers={
            "Authorization": f"Bearer {TOKEN}",
            "Content-Type": "application/json",
        },
    )
    started = time.perf_counter()
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            body = response.read().decode("utf-8")
            elapsed_ms = (time.perf_counter() - started) * 1000
            try:
                decoded: dict[str, Any] | str = json.loads(body)
            except json.JSONDecodeError:
                decoded = body
            return response.status, elapsed_ms, decoded
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", errors="replace")
        elapsed_ms = (time.perf_counter() - started) * 1000
        return exc.code, elapsed_ms, body


def request_e4(gateway: str, output: Path) -> None:
    status, elapsed_ms, body = post_chat(gateway, text="collector outage E4")
    health_ok = False
    try:
        with urllib.request.urlopen(
            f"{gateway.rstrip('/')}/healthz", timeout=3
        ) as response:
            health_ok = response.status == 200
    except Exception:
        health_ok = False

    choices_present = isinstance(body, dict) and bool(body.get("choices"))
    passed = status == 200 and health_ok and choices_present
    result = {
        "E4": {
            "status": "PASS" if passed else "FAIL",
            "evidence": {
                "http_status": status,
                "latency_ms": round(elapsed_ms, 3),
                "gateway_health": health_ok,
                "choices_present": choices_present,
                "collector_expected_state": "stopped",
            },
        }
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(json.dumps(result, indent=2))
    if not passed:
        raise RuntimeError("E4 failed")


def percentile(values: list[float], pct: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, int(round((len(ordered) - 1) * pct))))
    return ordered[index]


def load_e5(
    gateway: str,
    count: int,
    concurrency: int,
    output: Path,
) -> None:
    def one(i: int) -> tuple[int, float]:
        status, latency, _ = post_chat(
            gateway,
            text=f"collector-down load request {i}",
            timeout=15,
        )
        return status, latency

    statuses: list[int] = []
    latencies: list[float] = []
    started = time.perf_counter()
    with ThreadPoolExecutor(max_workers=concurrency) as pool:
        futures = [pool.submit(one, i) for i in range(count)]
        for future in as_completed(futures):
            status, latency = future.result()
            statuses.append(status)
            latencies.append(latency)
    total_s = time.perf_counter() - started

    health_ok = False
    try:
        with urllib.request.urlopen(
            f"{gateway.rstrip('/')}/healthz", timeout=3
        ) as response:
            health_ok = response.status == 200
    except Exception:
        health_ok = False

    successes = sum(status == 200 for status in statuses)
    survival_ok = successes == count and health_ok
    result = {
        "E5": {
            "status": "PARTIAL" if survival_ok else "FAIL",
            "evidence": {
                "requests": count,
                "concurrency": concurrency,
                "successes": successes,
                "gateway_health_after_load": health_ok,
                "elapsed_s": round(total_s, 3),
                "throughput_rps": round(count / total_s, 3) if total_s else None,
                "p50_ms": round(percentile(latencies, 0.50), 3),
                "p95_ms": round(percentile(latencies, 0.95), 3),
                "p99_ms": round(percentile(latencies, 0.99), 3),
                "collector_expected_state": "stopped",
            },
            "limitation": (
                "Request-path survival is verified, but strict bounded queue/"
                "memory behavior is not proven without exporter queue/drop metrics."
            ),
        }
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(json.dumps(result, indent=2))
    if not survival_ok:
        raise RuntimeError("E5 load survival failed")


def bench(
    gateway: str,
    count: int,
    warmup: int,
    mode: str,
    output: Path,
) -> None:
    for i in range(warmup):
        status, _, _ = post_chat(gateway, text=f"warmup {mode} {i}")
        if status != 200:
            raise RuntimeError(f"benchmark warmup failed with HTTP {status}")

    latencies: list[float] = []
    statuses: list[int] = []
    for i in range(count):
        status, latency, _ = post_chat(
            gateway, text=f"benchmark {mode} {i}", timeout=15
        )
        statuses.append(status)
        latencies.append(latency)

    passed = all(status == 200 for status in statuses)
    result = {
        "mode": mode,
        "requests": count,
        "warmup": warmup,
        "all_http_200": passed,
        "mean_ms": round(statistics.fmean(latencies), 3),
        "p50_ms": round(percentile(latencies, 0.50), 3),
        "p95_ms": round(percentile(latencies, 0.95), 3),
        "p99_ms": round(percentile(latencies, 0.99), 3),
        "min_ms": round(min(latencies), 3),
        "max_ms": round(max(latencies), 3),
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(json.dumps(result, indent=2))
    if not passed:
        raise RuntimeError(f"benchmark {mode} had non-200 responses")


def compare_bench(enabled: Path, disabled: Path, output: Path) -> None:
    on = json.loads(enabled.read_text(encoding="utf-8"))
    off = json.loads(disabled.read_text(encoding="utf-8"))

    def delta(key: str) -> float | None:
        base = float(off[key])
        if base == 0:
            return None
        return round((float(on[key]) - base) / base * 100.0, 3)

    result = {
        "E7": {
            "status": "PASS",
            "evidence": {
                "telemetry_enabled": on,
                "telemetry_disabled": off,
                "overhead_percent": {
                    "mean": delta("mean_ms"),
                    "p50": delta("p50_ms"),
                    "p95": delta("p95_ms"),
                    "p99": delta("p99_ms"),
                },
            },
            "decision_note": (
                "PASS means the baseline was measured reproducibly. "
                "No arbitrary acceptance threshold is applied before the baseline."
            ),
        }
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(json.dumps(result, indent=2))


def make_summary(artifacts: Path, output: Path) -> None:
    results: dict[str, Any] = {}
    for filename in [
        "e1-e2-e6.json",
        "e3-security.json",
        "e4-fail-open.json",
        "e5-load.json",
        "e7-performance.json",
    ]:
        path = artifacts / filename
        if path.exists():
            results.update(json.loads(path.read_text(encoding="utf-8")))

    lines = [
        "# Firewall-LLM OTel Prototype — Experiment Evidence",
        "",
        "| ID | Status | Meaning |",
        "|---|---|---|",
    ]
    meanings = {
        "E1": "Agent → Gateway W3C distributed trace",
        "E2": "Provider/model/latency correlation",
        "E3": "Explainable security block in same trace",
        "E4": "Gateway remains available without Collector",
        "E5": "Collector-down load survival / boundedness evidence",
        "E6": "Privacy sentinel absent from exported trace",
        "E7": "Telemetry on/off performance baseline measured",
    }
    for experiment in [f"E{i}" for i in range(1, 8)]:
        entry = results.get(experiment, {})
        lines.append(
            f"| {experiment} | {entry.get('status', 'MISSING')} | "
            f"{meanings[experiment]} |"
        )

    lines.extend(
        [
            "",
            "## Interpretation",
            "",
            "- E5 is intentionally PARTIAL when request-path survival succeeds but "
            "exporter queue/drop metrics are not yet available.",
            "- E7 PASS means measurement completed; product acceptance threshold "
            "must be calibrated from this baseline rather than invented upfront.",
            "",
            "## Raw evidence",
            "",
            "The accompanying JSON and Jaeger trace files are the authoritative "
            "machine-readable evidence for this run.",
        ]
    )
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(output.read_text(encoding="utf-8"))


def main() -> None:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("wait-url")
    p.add_argument("url")
    p.add_argument("--timeout", type=float, default=90)

    p = sub.add_parser("wait-trace")
    p.add_argument("--jaeger", default=DEFAULT_JAEGER)
    p.add_argument("--trace-id", required=True)
    p.add_argument("--require", nargs="*", default=[])
    p.add_argument("--output", type=Path, required=True)
    p.add_argument("--timeout", type=float, default=45)

    p = sub.add_parser("validate-normal")
    p.add_argument("--trace", type=Path, required=True)
    p.add_argument("--sentinel", required=True)
    p.add_argument("--output", type=Path, required=True)

    p = sub.add_parser("validate-security")
    p.add_argument("--trace", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)

    p = sub.add_parser("e4")
    p.add_argument("--gateway", default=DEFAULT_GATEWAY)
    p.add_argument("--output", type=Path, required=True)

    p = sub.add_parser("e5")
    p.add_argument("--gateway", default=DEFAULT_GATEWAY)
    p.add_argument("--count", type=int, default=1200)
    p.add_argument("--concurrency", type=int, default=8)
    p.add_argument("--output", type=Path, required=True)

    p = sub.add_parser("bench")
    p.add_argument("--gateway", default=DEFAULT_GATEWAY)
    p.add_argument("--count", type=int, default=100)
    p.add_argument("--warmup", type=int, default=10)
    p.add_argument("--mode", required=True)
    p.add_argument("--output", type=Path, required=True)

    p = sub.add_parser("compare-bench")
    p.add_argument("--enabled", type=Path, required=True)
    p.add_argument("--disabled", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)

    p = sub.add_parser("summary")
    p.add_argument("--artifacts", type=Path, required=True)
    p.add_argument("--output", type=Path, required=True)

    args = parser.parse_args()

    if args.cmd == "wait-url":
        wait_url(args.url, args.timeout)
    elif args.cmd == "wait-trace":
        wait_trace(
            args.jaeger,
            args.trace_id,
            args.require,
            args.output,
            args.timeout,
        )
    elif args.cmd == "validate-normal":
        result = validate_normal(args.trace, args.sentinel)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2), encoding="utf-8")
        print(json.dumps(result, indent=2))
    elif args.cmd == "validate-security":
        result = validate_security(args.trace)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(result, indent=2), encoding="utf-8")
        print(json.dumps(result, indent=2))
    elif args.cmd == "e4":
        request_e4(args.gateway, args.output)
    elif args.cmd == "e5":
        load_e5(args.gateway, args.count, args.concurrency, args.output)
    elif args.cmd == "bench":
        bench(args.gateway, args.count, args.warmup, args.mode, args.output)
    elif args.cmd == "compare-bench":
        compare_bench(args.enabled, args.disabled, args.output)
    elif args.cmd == "summary":
        make_summary(args.artifacts, args.output)
    else:  # pragma: no cover
        raise AssertionError(args.cmd)


if __name__ == "__main__":
    try:
        main()
    except Exception as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        raise

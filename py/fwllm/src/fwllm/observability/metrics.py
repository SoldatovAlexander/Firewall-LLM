"""Prometheus business metrics for the gateway."""

from __future__ import annotations

import os

from prometheus_client import (
    CollectorRegistry,
    Counter,
    Histogram,
    multiprocess,
    values,
)

# 0.1.1 capacity: with uvicorn --workers, each worker is its own process.
# When PROMETHEUS_MULTIPROC_DIR is set, metrics go through per-PID mmap
# files and /metrics aggregates all workers; otherwise everything stays on
# the global default registry (tests, single-process dev). The dir must be
# container-local (fresh on every start) so no stale worker files survive.
_MULTIPROC = "PROMETHEUS_MULTIPROC_DIR" in os.environ
if _MULTIPROC:
    values.ValueClass = values.MultiProcessValue()  # type: ignore[no-untyped-call]

REGISTRY = CollectorRegistry() if _MULTIPROC else None
if _MULTIPROC:
    multiprocess.MultiProcessCollector(REGISTRY)  # type: ignore[no-untyped-call]


def _counter(name: str, documentation: str, labelnames: list[str] | None = None) -> Counter:
    if _MULTIPROC:
        return Counter(name, documentation, labelnames or [], registry=REGISTRY)
    return Counter(name, documentation, labelnames or [])


def _histogram(name: str, documentation: str, labelnames: list[str]) -> Histogram:
    if _MULTIPROC:
        return Histogram(name, documentation, labelnames, registry=REGISTRY)
    return Histogram(name, documentation, labelnames)


def generate_metrics() -> bytes:
    """Render exposition format from the active registry."""
    from prometheus_client import generate_latest

    if _MULTIPROC and REGISTRY is not None:
        return generate_latest(REGISTRY)
    return generate_latest()

REQUESTS = Counter(
    "fw_requests_total",
    "Total chat completions processed",
    ["client", "provider", "model", "code"],
)

TOKENS = Counter(
    "fw_tokens_total",
    "Tokens processed",
    ["client", "provider", "model", "direction"],
)

DURATION = Histogram(
    "fw_request_duration_seconds",
    "Upstream request duration",
    ["provider", "model"],
)

# R12: audit storage write failures are visible; the request itself is
# unaffected (log + count policy).
AUDIT_ERRORS = Counter(
    "fw_audit_errors_total",
    "Audit storage write failures",
)


def observe_audit_error() -> None:
    AUDIT_ERRORS.inc()


def observe_request(
    *,
    client: str,
    provider: str,
    model: str,
    code: str,
    duration: float,
    prompt: int = 0,
    completion: int = 0,
) -> None:
    REQUESTS.labels(client=client, provider=provider, model=model, code=code).inc()
    if prompt:
        TOKENS.labels(
            client=client, provider=provider, model=model, direction="prompt"
        ).inc(prompt)
    if completion:
        TOKENS.labels(
            client=client, provider=provider, model=model, direction="completion"
        ).inc(completion)
    DURATION.labels(provider=provider, model=model).observe(duration)

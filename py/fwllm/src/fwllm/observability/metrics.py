"""Prometheus business metrics for the gateway."""

from __future__ import annotations

from prometheus_client import Counter, Histogram

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

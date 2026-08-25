"""Observability metrics tests."""

import prometheus_client

from fwllm.observability.metrics import observe_request


def _value(name: str, labels: dict[str, str]) -> float | None:
    return prometheus_client.REGISTRY.get_sample_value(name, labels)


def test_observe_request_increments_counter():
    labels = {
        "client": "mtest",
        "provider": "ptest",
        "model": "gtest",
        "code": "ok",
    }
    before = _value("fw_requests_total", labels) or 0
    observe_request(
        client="mtest",
        provider="ptest",
        model="gtest",
        code="ok",
        duration=0.25,
        prompt=7,
        completion=3,
    )
    assert (_value("fw_requests_total", labels) or 0) == before + 1
    token_labels = {
        "client": "mtest",
        "provider": "ptest",
        "model": "gtest",
    }
    assert (
        _value("fw_tokens_total", {**token_labels, "direction": "prompt"}) or 0
    ) >= 7
    assert (
        _value("fw_tokens_total", {**token_labels, "direction": "completion"}) or 0
    ) >= 3


def test_observe_request_records_latency_histogram():
    observe_request(
        client="mtest2", provider="ptest2", model="gtest2", code="ok", duration=0.5
    )
    count = _value(
        "fw_request_duration_seconds_count",
        {"provider": "ptest2", "model": "gtest2"},
    )
    assert count is not None and count >= 1

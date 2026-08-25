"""Gateway emits business metrics for /metrics endpoint."""

import prometheus_client
from fastapi.testclient import TestClient

from fwllm.app import create_app
from fwllm.config import Config, ProviderConfig
from tests.test_gateway import CLIENT_KEY, FakeProvider, _body, _headers


def test_gateway_request_updates_fw_metrics():
    cfg = Config(
        providers={"mock": ProviderConfig(base_url="http://mock.local/v1")},
        clients={CLIENT_KEY: "alice"},
    )
    app = create_app(cfg, providers={"mock": FakeProvider()})
    with TestClient(app) as c:
        before = prometheus_client.REGISTRY.get_sample_value(
            "fw_requests_total",
            {"client": "alice", "provider": "mock", "model": "gpt-4o", "code": "ok"},
        ) or 0
        r = c.post("/v1/chat/completions", json=_body(), headers=_headers())
        assert r.status_code == 200

        after = prometheus_client.REGISTRY.get_sample_value(
            "fw_requests_total",
            {"client": "alice", "provider": "mock", "model": "gpt-4o", "code": "ok"},
        ) or 0
        assert after == before + 1

        tokens = prometheus_client.REGISTRY.get_sample_value(
            "fw_tokens_total",
            {"client": "alice", "provider": "mock", "model": "gpt-4o",
             "direction": "prompt"},
        )
        assert tokens is not None and tokens >= 3


def test_gateway_upstream_error_metric_code():
    cfg = Config(
        providers={"mock": ProviderConfig(base_url="http://mock.local/v1")},
        clients={CLIENT_KEY: "alice"},
    )

    class FailingProvider(FakeProvider):
        async def chat(self, payload):  # type: ignore[override]
            from fwllm.providers.base import ProviderError

            raise ProviderError("boom")

    app = create_app(cfg, providers={"mock": FailingProvider()})
    with TestClient(app) as c:
        r = c.post("/v1/chat/completions", json=_body(), headers=_headers())
        assert r.status_code == 502
    value = prometheus_client.REGISTRY.get_sample_value(
        "fw_requests_total",
        {"client": "alice", "provider": "mock", "model": "gpt-4o",
         "code": "upstream_error"},
    )
    assert value is not None and value >= 1

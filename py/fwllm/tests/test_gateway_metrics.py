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


METRICS_TOKEN = "metrics-scrape-key"  # noqa: S105 - test fixture value


def _metrics_app() -> TestClient:
    cfg = Config(
        providers={"mock": ProviderConfig(base_url="http://mock.local/v1")},
        clients={CLIENT_KEY: "alice"},
        admin_clients={"admin-key-1": "admin"},
        metrics_tokens={METRICS_TOKEN: "prometheus"},
    )
    return TestClient(create_app(cfg, providers={"mock": FakeProvider()}))


def test_metrics_scoped_token_scrapes_without_admin():
    """R15: monitoring uses a least-privilege metrics token, not admin."""
    with _metrics_app() as c:
        r = c.get("/metrics", headers={"Authorization": f"Bearer {METRICS_TOKEN}"})
        assert r.status_code == 200
        assert "fw_requests_total" in r.text


def test_metrics_unknown_or_missing_token_is_401():
    with _metrics_app() as c:
        assert c.get("/metrics").status_code == 401
        assert (
            c.get("/metrics", headers={"Authorization": "Bearer nope"}).status_code
            == 401
        )


def test_metrics_client_token_stays_forbidden():
    """R15: ordinary client tokens must not scrape metrics."""
    with _metrics_app() as c:
        assert c.get("/metrics", headers=_headers()).status_code == 403


def test_multiproc_mode_aggregates_worker_files(tmp_path):
    """0.1.1 capacity: with PROMETHEUS_MULTIPROC_DIR the render path reads
    per-PID mmap files (uvicorn --workers) instead of process memory."""
    import os
    import subprocess
    import sys

    env = {
        **os.environ,
        "PROMETHEUS_MULTIPROC_DIR": str(tmp_path),
        "PYTHONPATH": "src",
    }
    code = (
        "from fwllm.observability.metrics import generate_metrics, observe_request;"
        "observe_request(client='w', provider='p', model='m', code='ok',"
        " duration=0.01, prompt=2, completion=3);"
        "out = generate_metrics().decode();"
        "assert 'fw_requests_total' in out, out;"
        "print('MULTIPROC_OK')"
    )
    proc = subprocess.run(  # noqa: S603
        [sys.executable, "-c", code],
        capture_output=True,
        text=True,
        env=env,
        cwd=".",
        timeout=60,
    )
    assert "MULTIPROC_OK" in proc.stdout, proc.stderr


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

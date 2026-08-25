"""Idempotent Grafana dashboard/datasource import tests (HTTP API mocked)."""

import json
from pathlib import Path
from typing import Any

import httpx
import respx

from fwllm.observability.grafana_import import ensure_datasource, import_dashboard

GRAFANA = "http://grafana.test:3000"

DASHBOARD: dict[str, Any] = {
    "uid": "fwllm-test",
    "title": "FWLLM Test",
    "schemaVersion": 39,
    "panels": [],
}


def _client() -> httpx.Client:
    return httpx.Client(base_url=GRAFANA)


@respx.mock
def test_import_dashboard_posts_with_overwrite():
    route = respx.post(f"{GRAFANA}/api/dashboards/db").respond(
        json={"status": "success", "uid": "fwllm-test"}
    )
    with _client() as c:
        result = import_dashboard(c, auth="Bearer t", dashboard=DASHBOARD)
    assert result["uid"] == "fwllm-test"
    body = json.loads(route.calls.last.request.content)
    assert body["overwrite"] is True
    assert body["dashboard"]["id"] is None
    assert body["dashboard"]["uid"] == "fwllm-test"
    assert route.calls.last.request.headers["authorization"] == "Bearer t"


@respx.mock
def test_ensure_datasource_creates_when_missing():
    respx.get(f"{GRAFANA}/api/datasources/uid/fwllm-prometheus").respond(status_code=404)
    route = respx.post(f"{GRAFANA}/api/datasources").respond(
        json={"datasource": {"uid": "fwllm-prometheus"}}
    )
    with _client() as c:
        assert ensure_datasource(c, auth="Bearer t", url="http://prometheus:9090") is True
    assert route.called


@respx.mock
def test_ensure_datasource_skips_when_exists():
    respx.get(f"{GRAFANA}/api/datasources/uid/fwllm-prometheus").respond(
        json={"uid": "fwllm-prometheus"}
    )
    post = respx.post(f"{GRAFANA}/api/datasources").respond(json={})
    with _client() as c:
        assert ensure_datasource(c, auth="Bearer t", url="http://prometheus:9090") is False
    assert not post.called


def test_bundled_dashboard_is_valid_and_references_metrics():
    path = (
        Path(__file__).resolve().parents[3]
        / "deploy"
        / "grafana"
        / "dashboards"
        / "fwllm-overview.json"
    )
    dash = json.loads(path.read_text(encoding="utf-8"))
    assert dash["uid"] == "fwllm-overview"
    text = json.dumps(dash)
    for metric in ("fw_requests_total", "fw_tokens_total"):
        assert metric in text

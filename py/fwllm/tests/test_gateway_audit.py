"""Gateway audit integration + admin API tests."""

import fakeredis.aioredis
from fastapi.testclient import TestClient

from fwllm.app import create_app
from fwllm.audit import AuditConfig, AuditLog
from fwllm.config import Config, ProviderConfig
from tests.test_gateway import CLIENT_KEY, FakeProvider, _body, _headers


def _app(tmp_path) -> tuple[TestClient, AuditLog]:
    cfg = Config(
        providers={"mock": ProviderConfig(base_url="http://mock.local/v1")},
        clients={CLIENT_KEY: "alice"},
        audit=AuditConfig(db_path=str(tmp_path / "audit.db")),
    )
    audit = AuditLog(cfg.audit)
    app = create_app(
        cfg,
        providers={"mock": FakeProvider()},
        metering=__import__("fwllm.metering", fromlist=["Metering"]).Metering(
            fakeredis.aioredis.FakeRedis(decode_responses=True)
        ),
        audit_log=audit,
    )
    return TestClient(app), audit


def test_successful_request_is_audited(tmp_path):
    client, audit = _app(tmp_path)
    with client:
        r = client.post("/v1/chat/completions", json=_body(), headers=_headers())
        assert r.status_code == 200
    rows = audit.search()
    assert len(rows) == 1
    assert rows[0]["code"] == "ok"
    assert rows[0]["provider"] == "mock"


def test_blocked_request_is_audited_with_reason(tmp_path):
    class BlockingProvider(FakeProvider):
        async def chat(self, payload):  # type: ignore[override]
            from fwllm.providers.base import BlockedError

            raise BlockedError("injection", reason="injection")

    cfg = Config(
        providers={"mock": ProviderConfig(base_url="http://mock.local/v1")},
        clients={CLIENT_KEY: "alice"},
        audit=AuditConfig(db_path=str(tmp_path / "audit.db")),
    )
    audit = AuditLog(cfg.audit)
    app = create_app(
        cfg,
        providers={"mock": BlockingProvider()},
        metering=__import__("fwllm.metering", fromlist=["Metering"]).Metering(
            fakeredis.aioredis.FakeRedis(decode_responses=True)
        ),
        audit_log=audit,
    )
    with TestClient(app) as client:
        client.post("/v1/chat/completions", json=_body(), headers=_headers())
    assert audit.search()[0]["code"] == "blocked"


def test_admin_api_requires_auth(tmp_path):
    client, _audit = _app(tmp_path)
    with client:
        assert client.get("/admin/audit").status_code == 401


def test_admin_api_returns_records_for_valid_key(tmp_path):
    client, _audit = _app(tmp_path)
    with client:
        client.post("/v1/chat/completions", json=_body(), headers=_headers())
        r = client.get("/admin/audit", headers=_headers())
        assert r.status_code == 200
        data = r.json()
    assert data["total"] >= 1
    assert data["records"][0]["client"] == "alice"


def test_disabled_audit_writes_nothing(tmp_path):
    cfg = Config(
        providers={"mock": ProviderConfig(base_url="http://mock.local/v1")},
        clients={CLIENT_KEY: "alice"},
        audit=AuditConfig(enabled=False, db_path=str(tmp_path / "audit.db")),
    )
    audit = AuditLog(cfg.audit)
    app = create_app(
        cfg,
        providers={"mock": FakeProvider()},
        metering=__import__("fwllm.metering", fromlist=["Metering"]).Metering(
            fakeredis.aioredis.FakeRedis(decode_responses=True)
        ),
        audit_log=audit,
    )
    with TestClient(app) as client:
        client.post("/v1/chat/completions", json=_body(), headers=_headers())
    assert audit.search() == []

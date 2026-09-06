"""R01: admin endpoints deny regular clients when admin_clients is empty.

Matrix: empty/nonempty admin list x alice/bob/admin/unknown token.
"""

import fakeredis.aioredis
from fastapi.testclient import TestClient

from fwllm.app import create_app
from fwllm.audit import AuditConfig, AuditLog
from fwllm.config import Config, ProviderConfig
from fwllm.metering import Metering
from tests.test_gateway import FakeProvider, _body

ALICE_KEY = "alice-key"
BOB_KEY = "bob-key"
ADMIN_KEY = "admin-key"


def _app(tmp_path, admin_clients=None):
    cfg = Config(
        providers={"mock": ProviderConfig(base_url="http://mock.local/v1")},
        clients={ALICE_KEY: "alice", BOB_KEY: "bob"},
        admin_clients=admin_clients or {},
        audit=AuditConfig(db_path=str(tmp_path / "audit.db")),
    )
    app = create_app(
        cfg,
        providers={"mock": FakeProvider()},
        metering=Metering(fakeredis.aioredis.FakeRedis(decode_responses=True)),
        audit_log=AuditLog(cfg.audit),
    )
    return TestClient(app)


def _seed(client):
    with client:
        client.post(
            "/v1/chat/completions",
            json=_body(),
            headers={"Authorization": f"Bearer {ALICE_KEY}"},
        )
        client.post(
            "/v1/chat/completions",
            json=_body(),
            headers={"Authorization": f"Bearer {BOB_KEY}"},
        )


def test_empty_admin_list_client_sees_only_own_records(tmp_path):
    client = _app(tmp_path)
    _seed(client)
    with client:
        r = client.get(
            "/admin/audit", headers={"Authorization": f"Bearer {ALICE_KEY}"}
        )
        assert r.status_code == 200
        records = r.json()["records"]
        assert records, "alice should see her own records"
        assert {rec["client"] for rec in records} == {"alice"}
        # ?client=bob must be ignored for non-admin
        r = client.get(
            "/admin/audit?client=bob",
            headers={"Authorization": f"Bearer {ALICE_KEY}"},
        )
        assert r.status_code == 200
        assert {rec["client"] for rec in r.json()["records"]} == {"alice"}


def test_empty_admin_list_metrics_denied_for_client(tmp_path):
    client = _app(tmp_path)
    with client:
        r = client.get(
            "/metrics", headers={"Authorization": f"Bearer {ALICE_KEY}"}
        )
        assert r.status_code == 403


def test_empty_admin_list_unknown_token_401(tmp_path):
    client = _app(tmp_path)
    with client:
        assert (
            client.get(
                "/admin/audit", headers={"Authorization": "Bearer nope"}
            ).status_code
            == 401
        )
        assert (
            client.get("/metrics", headers={"Authorization": "Bearer nope"}).status_code
            == 401
        )


def test_with_admin_list_admin_sees_all(tmp_path):
    client = _app(tmp_path, admin_clients={ADMIN_KEY: "admin"})
    _seed(client)
    with client:
        r = client.get(
            "/admin/audit", headers={"Authorization": f"Bearer {ADMIN_KEY}"}
        )
        assert r.status_code == 200
        assert {rec["client"] for rec in r.json()["records"]} == {"alice", "bob"}
        r = client.get(
            "/metrics", headers={"Authorization": f"Bearer {ADMIN_KEY}"}
        )
        assert r.status_code == 200


def test_with_admin_list_client_cannot_escalate(tmp_path):
    client = _app(tmp_path, admin_clients={ADMIN_KEY: "admin"})
    with client:
        r = client.get(
            "/metrics", headers={"Authorization": f"Bearer {ALICE_KEY}"}
        )
        assert r.status_code == 403


def test_startup_warns_when_no_admin_tokens(tmp_path, caplog):
    import logging

    with caplog.at_level(logging.WARNING):
        _app(tmp_path)
    assert any(
        "admin" in rec.message.lower() for rec in caplog.records
    ), "expected startup warning about disabled admin endpoints"

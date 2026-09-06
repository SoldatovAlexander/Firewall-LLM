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


def test_audit_row_carries_request_id(tmp_path):
    """R12: every final row carries the request id for correlation."""
    client, audit = _app(tmp_path)
    with client:
        r = client.post("/v1/chat/completions", json=_body(), headers=_headers())
        assert r.status_code == 200
    rows = audit.search()
    assert len(rows) == 1
    assert rows[0]["request_id"]


async def test_stream_disconnect_audited_as_cancelled(tmp_path):
    """R12: client abort mid-stream writes exactly one cancelled row.

    Drives the ASGI app directly: after the first body chunk the receive
    channel reports http.disconnect, so Starlette cancels the stream exactly
    like a real server would (httpx ASGI/TestClient buffer the whole body
    and can never deliver a mid-stream abort).
    """
    import asyncio
    import json as jsonlib
    import time
    from collections.abc import AsyncIterator
    from typing import Any

    release = asyncio.Event()

    class BlockingProvider(FakeProvider):
        async def chat_stream(
            self, payload: dict[str, Any]
        ) -> AsyncIterator[dict[str, Any]]:
            yield {"choices": [{"delta": {"content": "part1"}}]}
            # stay suspended mid-stream until the disconnect lands
            # (bare wait: wait_for would convert CancelledError to TimeoutError)
            await release.wait()
            yield {"choices": [{"delta": {"content": "part2"}}]}  # pragma: no cover

    import fakeredis.aioredis

    from fwllm.audit import AuditConfig, AuditLog
    from fwllm.config import Config, ProviderConfig
    from fwllm.metering import Metering

    cfg = Config(
        providers={"mock": ProviderConfig(base_url="http://mock.local/v1")},
        clients={CLIENT_KEY: "alice"},
        audit=AuditConfig(db_path=str(tmp_path / "audit.db")),
    )
    audit = AuditLog(cfg.audit)
    app = create_app(
        cfg,
        providers={"mock": BlockingProvider()},
        metering=Metering(fakeredis.aioredis.FakeRedis(decode_responses=True)),
        audit_log=audit,
    )

    request_body = jsonlib.dumps(_body(stream=True)).encode()
    body_sent = False
    first_chunk_seen = asyncio.Event()
    status_code: list[int] = []

    async def receive() -> dict[str, Any]:
        nonlocal body_sent
        if not body_sent:
            body_sent = True
            return {"type": "http.request", "body": request_body, "more_body": False}
        # Starlette polls this for disconnects while streaming.
        await first_chunk_seen.wait()
        return {"type": "http.disconnect"}

    async def send(message: dict[str, Any]) -> None:
        if message["type"] == "http.response.start":
            status_code.append(message["status"])
        elif message["type"] == "http.response.body" and message.get("body"):
            first_chunk_seen.set()

    scope = {
        "type": "http",
        "asgi": {"version": "3.0"},
        "http_version": "1.1",
        "method": "POST",
        "scheme": "http",
        "path": "/v1/chat/completions",
        "raw_path": b"/v1/chat/completions",
        "query_string": b"",
        "headers": [
            (b"authorization", _headers()["Authorization"].encode()),
            (b"content-type", b"application/json"),
        ],
        "server": ("test", 80),
        "client": ("test", 5000),
    }
    await asyncio.wait_for(app(scope, receive, send), timeout=15)
    assert status_code == [200]
    release.set()
    deadline = time.time() + 10
    rows = []
    while time.time() < deadline:
        rows = audit.search()
        if len(rows) == 1 and rows[0]["code"] == "cancelled":
            break
        await asyncio.sleep(0.1)
    assert len(rows) == 1
    assert rows[0]["code"] == "cancelled"


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

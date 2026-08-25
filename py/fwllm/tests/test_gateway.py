"""Gateway tests: unified API, auth, error mapping, streaming."""

import json
from collections.abc import AsyncIterator
from typing import Any

from fastapi.testclient import TestClient

from fwllm.app import create_app
from fwllm.config import Config, ProviderConfig, ServerConfig
from fwllm.providers.base import BlockedError, ProviderError


class FakeProvider:
    """Deterministic provider used by gateway tests."""

    def __init__(self, *, fail: bool = False, blocks: bool = False):
        self.fail = fail
        self.blocks = blocks
        self.calls: list[dict[str, Any]] = []

    async def chat(self, payload: dict[str, Any]) -> dict[str, Any]:
        self.calls.append(payload)
        if self.fail:
            raise ProviderError("upstream exploded")
        return {
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 1700000000,
            "model": payload["model"],
            "choices": [
                {
                    "index": 0,
                    "message": {"role": "assistant", "content": "Hi!"},
                    "finish_reason": "stop",
                }
            ],
            "usage": {
                "prompt_tokens": 3,
                "completion_tokens": 2,
                "total_tokens": 5,
            },
        }

    async def chat_stream(self, payload: dict[str, Any]) -> AsyncIterator[dict[str, Any]]:
        self.calls.append(payload)
        for piece in ("Hel", "lo!"):
            yield {
                "id": "chatcmpl-1",
                "object": "chat.completion.chunk",
                "created": 1700000000,
                "model": payload["model"],
                "choices": [
                    {"index": 0, "delta": {"content": piece}, "finish_reason": None}
                ],
            }


def _config(**kwargs: Any) -> Config:
    defaults: dict[str, Any] = {
        "server": ServerConfig(),
        "providers": {"mock": ProviderConfig(base_url="http://mock.local/v1")},
        "clients": {CLIENT_KEY: "alice"},
    }
    defaults.update(kwargs)
    return Config(**defaults)


def _client(provider: FakeProvider, cfg: Config | None = None) -> TestClient:
    app = create_app(cfg or _config(), providers={"mock": provider})
    return TestClient(app)


CLIENT_KEY = "secret-client-key"  # noqa: S105 - test fixture value


def _headers(token: str = CLIENT_KEY) -> dict[str, str]:
    return {"Authorization": f"Bearer {token}"}


def _body(**overrides: Any) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "model": "gpt-4o",
        "messages": [{"role": "user", "content": "hi"}],
    }
    payload.update(overrides)
    return payload


# --- health & metrics -------------------------------------------------------


def test_healthz():
    with _client(FakeProvider()) as c:
        r = c.get("/healthz")
        assert r.status_code == 200
        assert r.json() == {"status": "ok"}


def test_metrics_exposed():
    with _client(FakeProvider()) as c:
        assert c.get("/metrics").status_code == 200


# --- authentication ---------------------------------------------------------


def test_missing_auth_returns_401_contract_error():
    with _client(FakeProvider()) as c:
        r = c.post("/v1/chat/completions", json=_body())
        assert r.status_code == 401
        err = r.json()["error"]
        assert err["type"] == "authentication_error"


def test_invalid_key_returns_401():
    with _client(FakeProvider()) as c:
        r = c.post("/v1/chat/completions", json=_body(), headers=_headers("wrong"))
        assert r.status_code == 401
        assert r.json()["error"]["type"] == "authentication_error"


def test_no_clients_configured_rejects_everyone():
    cfg = _config(clients={})
    with _client(FakeProvider(), cfg) as c:
        assert c.post("/v1/chat/completions", json=_body(), headers=_headers()).status_code == 401


def test_valid_key_reaches_provider():
    p = FakeProvider()
    with _client(p) as c:
        r = c.post("/v1/chat/completions", json=_body(), headers=_headers())
        assert r.status_code == 200
        assert len(p.calls) == 1


# --- request validation ------------------------------------------------------


def test_invalid_body_maps_to_422_contract_error():
    with _client(FakeProvider()) as c:
        r = c.post(
            "/v1/chat/completions",
            json={"model": "gpt-4o"},
            headers=_headers(),
        )
        assert r.status_code == 422
        err = r.json()["error"]
        assert err["type"] == "invalid_request_error"


def test_empty_messages_rejected():
    with _client(FakeProvider()) as c:
        r = c.post("/v1/chat/completions", json=_body(messages=[]), headers=_headers())
        assert r.status_code == 422


# --- happy path --------------------------------------------------------------


def test_completion_passthrough_shape():
    with _client(FakeProvider()) as c:
        r = c.post("/v1/chat/completions", json=_body(), headers=_headers())
        data = r.json()
        assert data["object"] == "chat.completion"
        assert data["usage"]["total_tokens"] == 5
        assert data["choices"][0]["message"]["content"] == "Hi!"


# --- upstream failure --------------------------------------------------------


def test_provider_failure_maps_to_502():
    with _client(FakeProvider(fail=True)) as c:
        r = c.post("/v1/chat/completions", json=_body(), headers=_headers())
        assert r.status_code == 502
        assert r.json()["error"]["type"] == "upstream_error"


# --- inspector blocking ------------------------------------------------------


def test_blocked_request_maps_to_403():
    class BlockingProvider(FakeProvider):
        async def chat(self, payload: dict[str, Any]) -> dict[str, Any]:  # type: ignore[override]
            raise BlockedError("prompt injection detected", reason="injection")

    with _client(BlockingProvider()) as c:
        r = c.post("/v1/chat/completions", json=_body(), headers=_headers())
        assert r.status_code == 403
        err = r.json()["error"]
        assert err["type"] == "permission_error"
        assert err["details"]["reason"] == "injection"


# --- streaming ----------------------------------------------------------------


def test_streaming_sse_chunks_and_done():
    with _client(FakeProvider()) as c:
        with c.stream(
            "POST",
            "/v1/chat/completions",
            json=_body(stream=True),
            headers=_headers(),
        ) as r:
            assert r.status_code == 200
            assert r.headers["content-type"].startswith("text/event-stream")
            text = "".join(chunk for chunk in r.iter_text())
    events = [
        line.removeprefix("data: ") for line in text.splitlines() if line.startswith("data: ")
    ]
    assert events[-1] == "[DONE]"
    chunks = [json.loads(e) for e in events[:-1]]
    assert "".join(ch["choices"][0]["delta"].get("content", "") for ch in chunks) == "Hello!"


def test_stream_upstream_failure_emits_error_event():
    class FailingStreamProvider(FakeProvider):
        async def chat_stream(self, payload: dict[str, Any]) -> AsyncIterator[dict[str, Any]]:
            raise ProviderError("boom")
            yield {}  # pragma: no cover - makes this an async generator

    with _client(FailingStreamProvider()) as c:
        r = c.post("/v1/chat/completions", json=_body(stream=True), headers=_headers())
    assert "upstream_error" in r.text

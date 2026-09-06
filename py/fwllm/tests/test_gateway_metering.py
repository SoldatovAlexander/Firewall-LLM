"""Gateway + metering integration tests."""

from typing import Any

import fakeredis.aioredis
from fastapi.testclient import TestClient

from fwllm.app import create_app
from fwllm.config import Config, ProviderConfig, Quotas, ServerConfig
from fwllm.metering import Metering
from tests.test_gateway import CLIENT_KEY, FakeProvider, _body, _headers


def _app(quotas: Quotas | None = None) -> tuple[TestClient, Any]:
    redis = fakeredis.aioredis.FakeRedis(decode_responses=True)
    cfg = Config(
        server=ServerConfig(),
        providers={"mock": ProviderConfig(base_url="http://mock.local/v1")},
        clients={CLIENT_KEY: "alice"},
        quotas=quotas or Quotas(),
    )
    provider = FakeProvider()
    metering = Metering(
        redis, quotas=(quotas or Quotas()).model_dump(exclude_none=True)
    )
    app = create_app(cfg, providers={"mock": provider}, metering=metering)
    return TestClient(app), redis


def _today() -> str:
    from datetime import UTC, datetime

    return datetime.now(UTC).strftime("%Y%m%d")


async def test_usage_recorded_after_completion():
    client, redis = _app()
    with client:
        r = client.post(
            "/v1/chat/completions", json=_body(), headers=_headers()
        )
        assert r.status_code == 200
    # FakeProvider usage totals 5 tokens
    day = _today()
    assert int(await redis.get(f"fwllm:c:tokens:alice:{day}") or 0) == 5
    assert int(await redis.get(f"fwllm:p:tokens:mock:{day}") or 0) == 5



async def test_fail_closed_rejects_when_redis_unreachable():
    from fwllm.config import Quotas

    class BrokenRedis:
        async def ping(self): raise Exception("redis down")
        async def get(self, k): raise Exception("redis down")
        async def incrby(self, k, a): raise Exception("redis down")
        async def expire(self, k, s): raise Exception("redis down")

    cfg = Config(
        server=ServerConfig(),
        providers={"mock": ProviderConfig(base_url="http://mock.local/v1")},
        clients={CLIENT_KEY: "alice"},
        quotas=Quotas(client_tokens_per_day=10, backend_fail_closed=True),
    )
    metering = Metering(
        BrokenRedis(), quotas={"client_tokens_per_day": 10}, backend_fail_closed=True
    )
    app = create_app(cfg, providers={"mock": FakeProvider()}, metering=metering)
    from fastapi.testclient import TestClient as TC
    with TC(app) as c:
        r = c.post("/v1/chat/completions", json=_body(), headers=_headers())
        assert r.status_code in (502, 503)

class NoUsageStreamProvider(FakeProvider):
    """R03: yields text chunks with no usage object at all."""

    async def chat_stream(self, payload: dict[str, Any]):
        self.calls.append(payload)
        for piece in ("Hel", "lo!"):
            yield {"choices": [{"delta": {"content": piece}}]}


class UsageStreamProvider(FakeProvider):
    """R03: usage arrives in a terminal chunk with empty choices."""

    async def chat_stream(self, payload: dict[str, Any]):
        self.calls.append(payload)
        yield {"choices": [{"delta": {"content": "Hi"}}]}
        yield {
            "choices": [],
            "usage": {"prompt_tokens": 4, "completion_tokens": 2},
        }


def _stream_app(
    quotas: Quotas, provider: FakeProvider
) -> tuple[TestClient, Any, list, FakeProvider]:
    redis = fakeredis.aioredis.FakeRedis(decode_responses=True)
    cfg = Config(
        server=ServerConfig(),
        providers={"mock": ProviderConfig(base_url="http://mock.local/v1")},
        clients={CLIENT_KEY: "alice"},
        quotas=quotas,
    )
    metering = Metering(redis, quotas=quotas.model_dump(exclude_none=True))
    events: list = []
    metering.subscribe(events.append)
    app = create_app(cfg, providers={"mock": provider}, metering=metering)
    return TestClient(app), redis, events, provider


def _stream_text(client: TestClient, **overrides: Any) -> tuple[int, str]:
    body = _body(stream=True)
    body.update(overrides)
    with client.stream(
        "POST", "/v1/chat/completions", json=body, headers=_headers()
    ) as r:
        status = r.status_code
        text = "".join(chunk for chunk in r.iter_text()) if status == 200 else ""
    return status, text


async def test_stream_without_usage_counts_request_second_is_429():
    """R03: SSE without usage must still consume the request quota."""
    quotas = Quotas(client_requests_per_day=1)
    client, redis, _events, _provider = _stream_app(
        quotas, NoUsageStreamProvider()
    )
    day = _today()
    with client:
        assert _stream_text(client)[0] == 200
        assert _stream_text(client)[0] == 429
    assert int(await redis.get(f"fwllm:c:req:alice:{day}") or 0) == 1


async def test_stream_without_usage_estimates_tokens_with_source():
    """R03: missing usage falls back to a marked estimate, never zero-count."""
    client, _redis, events, _provider = _stream_app(Quotas(), NoUsageStreamProvider())
    with client:
        assert _stream_text(client)[0] == 200
    spent = [e for e in events if e.name == "tokens_spent"]
    assert len(spent) == 1
    assert spent[0].data["usage_source"] == "estimated"
    assert spent[0].data["total_tokens"] > 0


async def test_stream_with_usage_keeps_upstream_numbers():
    """R03: provider usage wins and is marked as upstream."""
    client, _redis, events, _provider = _stream_app(Quotas(), UsageStreamProvider())
    with client:
        assert _stream_text(client)[0] == 200
    spent = [e for e in events if e.name == "tokens_spent"]
    assert len(spent) == 1
    assert spent[0].data["usage_source"] == "upstream"
    assert spent[0].data["total_tokens"] == 6


async def test_stream_requests_include_usage_upstream():
    """R03: gateway asks supporting providers for usage; user options kept."""
    provider = NoUsageStreamProvider()
    client, _redis, _events, _p = _stream_app(Quotas(), provider)
    with client:
        assert _stream_text(client, stream_options={"foo": "bar"})[0] == 200
    sent = provider.calls[0]
    assert sent["stream_options"]["include_usage"] is True
    assert sent["stream_options"]["foo"] == "bar"


async def test_stream_disconnect_still_records_request():
    """R03: client abort mid-stream must not lose the accounting.

    Drives raw ASGI so http.disconnect genuinely cancels the stream
    (TestClient/httpx-ASGI would let the producer finish first).
    """
    import asyncio
    import json as jsonlib
    from collections.abc import AsyncIterator

    release = asyncio.Event()

    class BlockingNoUsageProvider(NoUsageStreamProvider):
        async def chat_stream(
            self, payload: dict[str, Any]
        ) -> AsyncIterator[dict[str, Any]]:
            async for chunk in super().chat_stream(payload):
                yield chunk
                # stay suspended mid-stream until the disconnect lands
                await release.wait()
                return

    quotas = Quotas()
    redis = fakeredis.aioredis.FakeRedis(decode_responses=True)
    cfg = Config(
        server=ServerConfig(),
        providers={"mock": ProviderConfig(base_url="http://mock.local/v1")},
        clients={CLIENT_KEY: "alice"},
        quotas=quotas,
    )
    metering = Metering(redis, quotas=quotas.model_dump(exclude_none=True))
    app = create_app(cfg, providers={"mock": BlockingNoUsageProvider()}, metering=metering)

    request_body = jsonlib.dumps(_body(stream=True)).encode()
    body_sent = False
    first_chunk_seen = asyncio.Event()
    status_code: list[int] = []

    async def receive() -> dict[str, Any]:
        nonlocal body_sent
        if not body_sent:
            body_sent = True
            return {"type": "http.request", "body": request_body, "more_body": False}
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
    day = _today()
    assert int(await redis.get(f"fwllm:c:req:alice:{day}") or 0) == 1


async def test_concurrent_requests_single_upstream_call():
    """R05 acceptance: 20 concurrent streams with 1 slot → 1 upstream call."""
    import asyncio

    import httpx

    calls = 0

    class SlowProvider(FakeProvider):
        async def chat_stream(self, payload: dict[str, Any]):
            nonlocal calls
            calls += 1
            await asyncio.sleep(0.05)
            yield {"choices": [{"delta": {"content": "Hi"}}]}

    quotas = Quotas(client_requests_per_day=1)
    redis = fakeredis.aioredis.FakeRedis(decode_responses=True)
    cfg = Config(
        server=ServerConfig(),
        providers={"mock": ProviderConfig(base_url="http://mock.local/v1")},
        clients={CLIENT_KEY: "alice"},
        quotas=quotas,
    )
    metering = Metering(redis, quotas=quotas.model_dump(exclude_none=True))
    app = create_app(cfg, providers={"mock": SlowProvider()}, metering=metering)
    transport = httpx.ASGITransport(app=app)
    async with httpx.AsyncClient(
        transport=transport, base_url="http://test"
    ) as http:
        responses = await asyncio.gather(
            *[
                http.post(
                    "/v1/chat/completions",
                    json=_body(stream=True),
                    headers=_headers(),
                )
                for _ in range(20)
            ]
        )
    statuses = sorted(r.status_code for r in responses)
    assert statuses == [200] + [429] * 19
    assert calls == 1


async def test_provider_error_keeps_prompt_charge_refunds_completion():
    """R05: upstream failure settles the reserve (prompt kept, cap refunded)."""
    from fwllm.providers.base import ProviderError

    class FailingProvider(FakeProvider):
        async def chat(self, payload: dict[str, Any]) -> dict[str, Any]:
            raise ProviderError("upstream exploded")

    quotas = Quotas(client_tokens_per_day=100_000)
    redis = fakeredis.aioredis.FakeRedis(decode_responses=True)
    cfg = Config(
        server=ServerConfig(),
        providers={"mock": ProviderConfig(base_url="http://mock.local/v1")},
        clients={CLIENT_KEY: "alice"},
        quotas=quotas,
    )
    metering = Metering(redis, quotas=quotas.model_dump(exclude_none=True))
    app = create_app(cfg, providers={"mock": FailingProvider()}, metering=metering)
    prompt = "x" * 400  # prompt_est = 100
    with TestClient(app) as c:
        r = c.post(
            "/v1/chat/completions",
            json=_body(messages=[{"role": "user", "content": prompt}]),
            headers=_headers(),
        )
        assert r.status_code == 502
    day = _today()
    # only the prompt estimate stays; the completion cap was refunded
    assert int(await redis.get(f"fwllm:c:tokens:alice:{day}") or 0) == 100
    assert int(await redis.get(f"fwllm:c:req:alice:{day}") or 0) == 1


async def test_quota_exceeded_returns_429_contract_error():
    quotas = Quotas(client_tokens_per_day=3)
    client, redis = _app(quotas)
    day = _today()
    with client:
        await redis.set(f"fwllm:c:tokens:alice:{day}", "10")
        r = client.post("/v1/chat/completions", json=_body(), headers=_headers())
        assert r.status_code == 429
        assert r.json()["error"]["type"] == "rate_limit_error"

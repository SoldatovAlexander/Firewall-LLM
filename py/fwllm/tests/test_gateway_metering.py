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


async def test_quota_exceeded_returns_429_contract_error():
    quotas = Quotas(client_tokens_per_day=3)
    client, redis = _app(quotas)
    day = _today()
    with client:
        await redis.set(f"fwllm:c:tokens:alice:{day}", "10")
        r = client.post("/v1/chat/completions", json=_body(), headers=_headers())
        assert r.status_code == 429
        assert r.json()["error"]["type"] == "rate_limit_error"

"""Metering tests: token/request counters, quotas, policy events."""

from datetime import UTC, datetime
from typing import Any

import fakeredis.aioredis
import pytest

from fwllm.metering import Event, Metering, QuotaExceeded


def _clock(day: int) -> Any:
    return lambda: datetime(2026, 8, day, tzinfo=UTC)


@pytest.fixture
def redis() -> Any:
    return fakeredis.aioredis.FakeRedis(decode_responses=True)


async def test_record_increments_daily_counters(redis: Any) -> None:
    m = Metering(redis, clock=_clock(25))
    await m.record(
        client="alice", provider="p1", model="gpt-4o", prompt=10, completion=5
    )
    assert await redis.get("fwllm:c:tokens:alice:20260825") == "15"
    assert await redis.get("fwllm:c:req:alice:20260825") == "1"
    assert await redis.get("fwllm:p:tokens:p1:20260825") == "15"
    assert await redis.get("fwllm:m:tokens:gpt-4o:20260825") == "15"


async def test_separate_days_use_separate_buckets(redis: Any) -> None:
    await Metering(redis, clock=_clock(25)).record(
        client="alice", provider="p", model="m", prompt=7, completion=0
    )
    await Metering(redis, clock=_clock(26)).record(
        client="alice", provider="p", model="m", prompt=3, completion=0
    )
    assert await redis.get("fwllm:c:tokens:alice:20260825") == "7"
    assert await redis.get("fwllm:c:tokens:alice:20260826") == "3"


async def test_requests_counted_per_call(redis: Any) -> None:
    m = Metering(redis, clock=_clock(25))
    for _ in range(3):
        await m.record(client="bob", provider="p", model="m", prompt=1, completion=1)
    assert await redis.get("fwllm:c:req:bob:20260825") == "3"


async def test_token_quota_exceeded_raises(redis: Any) -> None:
    quotas = {"client_tokens_per_day": 10}
    m = Metering(redis, quotas=quotas, clock=_clock(25))
    await m.record(client="alice", provider="p", model="m", prompt=8, completion=2)
    with pytest.raises(QuotaExceeded) as exc_info:
        await m.check_client("alice")
    assert exc_info.value.limit == 10


async def test_under_quota_passes_and_request_quota_works(redis: Any) -> None:
    quotas = {"client_tokens_per_day": 100, "client_requests_per_day": 2}
    m = Metering(redis, quotas=quotas, clock=_clock(25))
    await m.record(client="carol", provider="p", model="m", prompt=1, completion=1)
    await m.check_client("carol")
    await m.record(client="carol", provider="p", model="m", prompt=1, completion=1)
    await m.record(client="carol", provider="p", model="m", prompt=1, completion=1)
    with pytest.raises(QuotaExceeded):
        await m.check_client("carol")


async def test_no_quotas_configured_never_exceeds(redis: Any) -> None:
    m = Metering(redis, clock=_clock(25))
    await m.record(client="dave", provider="p", model="m", prompt=999999, completion=0)
    await m.check_client("dave")


async def test_events_published(redis: Any) -> None:
    seen: list[Event] = []
    m = Metering(
        redis,
        quotas={"client_tokens_per_day": 5},
        clock=_clock(25),
        subscribers=[seen.append],
    )
    await m.record(client="eve", provider="p1", model="m", prompt=3, completion=2)
    assert any(e.name == "tokens_spent" for e in seen)
    spent = next(e for e in seen if e.name == "tokens_spent")
    assert spent.data["client"] == "eve"
    assert spent.data["total_tokens"] == 5
    with pytest.raises(QuotaExceeded):
        await m.check_client("eve")
    assert any(e.name == "quota_exceeded" for e in seen)

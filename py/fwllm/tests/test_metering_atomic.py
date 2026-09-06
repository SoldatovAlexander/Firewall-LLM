"""R05: atomic admission (check-and-reserve) and idempotent settle."""

import asyncio
from typing import Any

import fakeredis.aioredis
import pytest

from fwllm.config import Quotas
from fwllm.metering import Metering, QuotaExceeded


def _metering(quotas: Quotas) -> tuple[Metering, Any]:
    redis = fakeredis.aioredis.FakeRedis(decode_responses=True)
    return (
        Metering(redis, quotas=quotas.model_dump(exclude_none=True)),
        redis,
    )


def _today() -> str:
    from datetime import UTC, datetime

    return datetime.now(UTC).strftime("%Y%m%d")


async def test_admit_reserves_settle_refunds_and_is_idempotent():
    m, redis = _metering(Quotas(client_tokens_per_day=100))
    day = _today()
    rsv = await m.admit(
        client="a", provider="p", model="m", prompt_est=10, completion_cap=20
    )
    assert int(await redis.get(f"fwllm:c:tokens:a:{day}") or 0) == 30
    assert int(await redis.get(f"fwllm:c:req:a:{day}") or 0) == 1
    await m.settle(rsv, prompt=5, completion=5)
    assert int(await redis.get(f"fwllm:c:tokens:a:{day}") or 0) == 10
    # repeat settle is a no-op (provider retries, double finalize)
    await m.settle(rsv, prompt=5, completion=5)
    assert int(await redis.get(f"fwllm:c:tokens:a:{day}") or 0) == 10


async def test_concurrent_admit_single_request_slot():
    """R05 acceptance (single process): 20 concurrent admits, 1 slot."""
    m, redis = _metering(Quotas(client_requests_per_day=1))
    day = _today()
    results = await asyncio.gather(
        *[
            m.admit(
                client="a", provider="p", model="m", prompt_est=1, completion_cap=1
            )
            for _ in range(20)
        ],
        return_exceptions=True,
    )
    assert sum(1 for r in results if not isinstance(r, BaseException)) == 1
    assert all(
        isinstance(r, QuotaExceeded) for r in results if isinstance(r, BaseException)
    )
    assert int(await redis.get(f"fwllm:c:req:a:{day}") or 0) == 1


async def test_admit_token_quota_accounts_full_reserve():
    m, _redis = _metering(Quotas(client_tokens_per_day=25))
    with pytest.raises(QuotaExceeded):
        await m.admit(
            client="a", provider="p", model="m", prompt_est=10, completion_cap=20
        )
    # exact fit is admitted
    rsv = await m.admit(
        client="a", provider="p", model="m", prompt_est=10, completion_cap=15
    )
    await m.settle(rsv, prompt=10, completion=15)


async def test_settle_charges_overrun():
    """Actual usage above the reserve is charged, not silently dropped."""
    m, redis = _metering(Quotas(client_tokens_per_day=1000))
    day = _today()
    rsv = await m.admit(
        client="a", provider="p", model="m", prompt_est=10, completion_cap=20
    )
    await m.settle(rsv, prompt=50, completion=50)
    assert int(await redis.get(f"fwllm:c:tokens:a:{day}") or 0) == 100

"""Metering: token/request accounting, quotas, policy events.

All counters are daily buckets in Redis, keyed by client/provider/model.
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass, field
from datetime import UTC, datetime
from typing import Any, Protocol


class QuotaExceeded(Exception):
    def __init__(self, message: str, *, limit: int, scope: str):
        super().__init__(message)
        self.limit = limit
        self.scope = scope


@dataclass(frozen=True)
class Event:
    """Facts published for the policy engine (phase 7) and metrics."""

    name: str  # tokens_spent | quota_exceeded
    data: dict[str, Any] = field(default_factory=dict)


class Subscriber(Protocol):
    def __call__(self, event: Event) -> None: ...


class Metering:
    def __init__(
        self,
        redis: Any,
        quotas: dict[str, int] | None = None,
        *,
        clock: Callable[[], datetime] | None = None,
        subscribers: list[Subscriber] | None = None,
    ):
        self._redis = redis
        self._quotas = quotas or {}
        self._clock = clock or (lambda: datetime.now(UTC))
        self._subscribers = list(subscribers or [])

    def subscribe(self, subscriber: Subscriber) -> None:
        self._subscribers.append(subscriber)

    def _day(self) -> str:
        return self._clock().strftime("%Y%m%d")

    def _publish(self, event: Event) -> None:
        for subscriber in self._subscribers:
            subscriber(event)

    @staticmethod
    async def _incr(redis: Any, key: str, amount: int = 1) -> int:
        value: int = await redis.incrby(key, amount)
        await redis.expire(key, 60 * 60 * 48)  # keep 2 days of daily buckets
        return value

    async def check_client(self, client_id: str) -> None:
        day = self._day()
        token_limit = self._quotas.get("client_tokens_per_day")
        if token_limit is not None:
            used = int(await self._redis.get(f"fwllm:c:tokens:{client_id}:{day}") or 0)
            if used >= token_limit:
                self._publish(
                    Event(
                        "quota_exceeded",
                        {"client": client_id, "scope": "tokens", "limit": token_limit},
                    )
                )
                raise QuotaExceeded(
                    f"daily token quota exceeded ({used}/{token_limit})",
                    limit=token_limit,
                    scope="tokens",
                )
        request_limit = self._quotas.get("client_requests_per_day")
        if request_limit is not None:
            used = int(await self._redis.get(f"fwllm:c:req:{client_id}:{day}") or 0)
            if used >= request_limit:
                self._publish(
                    Event(
                        "quota_exceeded",
                        {"client": client_id, "scope": "requests", "limit": request_limit},
                    )
                )
                raise QuotaExceeded(
                    f"daily request quota exceeded ({used}/{request_limit})",
                    limit=request_limit,
                    scope="requests",
                )

    async def record(
        self,
        *,
        client: str,
        provider: str,
        model: str,
        prompt: int,
        completion: int,
    ) -> None:
        day = self._day()
        total = prompt + completion
        await self._incr(self._redis, f"fwllm:c:tokens:{client}:{day}", total)
        await self._incr(self._redis, f"fwllm:c:req:{client}:{day}")
        await self._incr(self._redis, f"fwllm:p:tokens:{provider}:{day}", total)
        await self._incr(self._redis, f"fwllm:p:req:{provider}:{day}")
        await self._incr(self._redis, f"fwllm:m:tokens:{model}:{day}", total)
        self._publish(
            Event(
                "tokens_spent",
                {
                    "client": client,
                    "provider": provider,
                    "model": model,
                    "total_tokens": total,
                },
            )
        )

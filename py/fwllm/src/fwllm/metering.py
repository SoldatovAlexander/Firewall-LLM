"""Metering: token/request accounting, quotas, policy events.

All counters are daily buckets in Redis, keyed by client/provider/model.
"""

from __future__ import annotations

import asyncio
import uuid
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
        backend_fail_closed: bool = False,
    ):
        self._redis = redis
        self._quotas = quotas or {}
        self._clock = clock or (lambda: datetime.now(UTC))
        self._subscribers = list(subscribers or [])
        self._backend_fail_closed = backend_fail_closed
        # R05: guards the non-scripting fallback below (single-process
        # correctness; cross-process atomicity requires real Redis + Lua).
        self._admit_lock = asyncio.Lock()
        self._fallback_open: dict[str, Reservation] = {}
        self._fallback_settled: set[str] = set()

    def subscribe(self, subscriber: Subscriber) -> None:
        self._subscribers.append(subscriber)

    def _day(self) -> str:
        return self._clock().strftime("%Y%m%d")

    def _publish(self, event: Event) -> None:
        for subscriber in self._subscribers:
            subscriber(event)

    async def _ensure_ready(self) -> None:
        """When fail-closed, verify the backend is reachable before serving."""
        if self._backend_fail_closed:
            await self._redis.ping()

    @staticmethod
    async def _incr(redis: Any, key: str, amount: int = 1) -> int:
        value: int = await redis.incrby(key, amount)
        await redis.expire(key, 60 * 60 * 48)  # keep 2 days of daily buckets
        return value

    async def check_client(self, client_id: str) -> None:
        await self._ensure_ready()
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

    async def check_provider(self, provider: str) -> None:
        await self._ensure_ready()
        day = self._day()
        provider_limit = self._quotas.get("provider_tokens_per_day")
        if provider_limit is not None:
            used = int(await self._redis.get(f"fwllm:p:tokens:{provider}:{day}") or 0)
            if used >= provider_limit:
                self._publish(
                    Event(
                        "quota_exceeded",
                        {"provider": provider, "scope": "provider_tokens", "limit": provider_limit},
                    )
                )
                raise QuotaExceeded(
                    f"daily provider token quota exceeded ({used}/{provider_limit}) for {provider}",
                    limit=provider_limit,
                    scope="provider_tokens",
                )

    def _reservation_keys(self, rsv: Reservation) -> tuple[list[str], str]:
        day = self._day()
        keys = [
            f"fwllm:c:tokens:{rsv.client}:{day}",
            f"fwllm:c:req:{rsv.client}:{day}",
            f"fwllm:p:tokens:{rsv.provider}:{day}",
            f"fwllm:p:req:{rsv.provider}:{day}",
            f"fwllm:m:tokens:{rsv.model}:{day}",
        ]
        return keys, f"fwllm:rsv:{rsv.id}"

    @staticmethod
    def _limit(value: int | None) -> int:
        return -1 if value is None else value

    def _raise_for_admit_result(self, result: Any, rsv: Reservation) -> None:
        status, used = str(result[0]), int(result[1])
        if status == "ok":
            return
        scope_limits = {
            "tokens": ("tokens", self._quotas.get("client_tokens_per_day")),
            "requests": ("requests", self._quotas.get("client_requests_per_day")),
            "provider_tokens": (
                "provider_tokens",
                self._quotas.get("provider_tokens_per_day"),
            ),
        }
        scope, limit = scope_limits[status]
        self._publish(
            Event(
                "quota_exceeded",
                {
                    "client": rsv.client,
                    "provider": rsv.provider,
                    "scope": scope,
                    "limit": limit,
                },
            )
        )
        raise QuotaExceeded(
            f"daily {scope} quota exceeded ({used}/{limit})",
            limit=int(limit or 0),
            scope=scope,
        )

    async def admit(
        self,
        *,
        client: str,
        provider: str,
        model: str,
        prompt_est: int,
        completion_cap: int,
    ) -> Reservation:
        """Atomically check quotas and reserve budget (R05).

        Reserve = prompt estimate + completion cap actually handed to the
        adapter. Raises QuotaExceeded without consuming budget. Uses a Redis
        Lua script when the backend supports scripting; otherwise falls back
        to a process-local locked section (correct for a single process —
        cross-process admission requires real Redis).
        """
        await self._ensure_ready()
        rsv = Reservation(
            id=uuid.uuid4().hex,
            client=client,
            provider=provider,
            model=model,
            prompt_est=max(0, prompt_est),
            completion_cap=max(0, completion_cap),
        )
        keys, rsv_key = self._reservation_keys(rsv)
        args = [
            self._limit(self._quotas.get("client_tokens_per_day")),
            self._limit(self._quotas.get("client_requests_per_day")),
            self._limit(self._quotas.get("provider_tokens_per_day")),
            rsv.reserved_total,
            "open",
            _RSV_TTL_SECS,
            _BUCKET_TTL_SECS,
        ]
        try:
            result = await self._redis.eval(ADMIT_LUA, len(keys) + 1, *keys, rsv_key, *args)
        except Exception as exc:
            if "unknown command" not in str(exc).lower():
                raise
            return await self._admit_fallback(rsv)
        self._raise_for_admit_result(result, rsv)
        return rsv

    async def _admit_fallback(self, rsv: Reservation) -> Reservation:
        """Single-process atomic admission for non-scripting backends."""
        async with self._admit_lock:
            keys, _ = self._reservation_keys(rsv)
            ctok = int(await self._redis.get(keys[0]) or 0)
            creq = int(await self._redis.get(keys[1]) or 0)
            ptok = int(await self._redis.get(keys[2]) or 0)
            checks = [
                ("tokens", ctok, self._quotas.get("client_tokens_per_day")),
                ("requests", creq, self._quotas.get("client_requests_per_day")),
                ("provider_tokens", ptok, self._quotas.get("provider_tokens_per_day")),
            ]
            for scope, used, limit in checks:
                need = 1 if scope == "requests" else rsv.reserved_total
                if limit is not None and used + need > limit:
                    self._publish(
                        Event(
                            "quota_exceeded",
                            {
                                "client": rsv.client,
                                "provider": rsv.provider,
                                "scope": scope,
                                "limit": limit,
                            },
                        )
                    )
                    raise QuotaExceeded(
                        f"daily {scope} quota exceeded ({used}/{limit})",
                        limit=limit,
                        scope=scope,
                    )
            await self._incr(self._redis, keys[0], rsv.reserved_total)
            await self._incr(self._redis, keys[1])
            await self._incr(self._redis, keys[2], rsv.reserved_total)
            await self._incr(self._redis, keys[3])
            await self._incr(self._redis, keys[4], rsv.reserved_total)
            self._fallback_open[rsv.id] = rsv
            return rsv

    async def settle(
        self,
        rsv: Reservation,
        *,
        prompt: int,
        completion: int,
        usage_source: str = "upstream",
    ) -> None:
        """Reconcile a reservation with actual usage, exactly once (R05).

        Refunds unused reserve or charges overrun on token counters; request
        counters are untouched (the admitted request stays counted). Repeat
        calls and post-expiry calls are no-ops; the tokens_spent event fires
        only for the effective settle.
        """
        delta = (prompt + completion) - rsv.reserved_total
        keys, rsv_key = self._reservation_keys(rsv)
        token_keys = [keys[0], keys[2], keys[4]]
        try:
            applied = await self._redis.eval(
                SETTLE_LUA, 4, rsv_key, *token_keys, delta, _BUCKET_TTL_SECS
            )
            applied = int(applied) == 1
        except Exception as exc:
            if "unknown command" not in str(exc).lower():
                raise
            applied = await self._settle_fallback(rsv, delta)
        if not applied:
            return
        self._publish(
            Event(
                "tokens_spent",
                {
                    "client": rsv.client,
                    "provider": rsv.provider,
                    "model": rsv.model,
                    "total_tokens": prompt + completion,
                    "usage_source": usage_source,
                },
            )
        )

    async def _settle_fallback(self, rsv: Reservation, delta: int) -> bool:
        async with self._admit_lock:
            if rsv.id in self._fallback_settled or rsv.id not in self._fallback_open:
                return False
            self._fallback_settled.add(rsv.id)
            del self._fallback_open[rsv.id]
            keys, _ = self._reservation_keys(rsv)
            for key in (keys[0], keys[2], keys[4]):
                current = int(await self._redis.get(key) or 0)
                await self._redis.set(key, max(0, current + delta))
                await self._redis.expire(key, _BUCKET_TTL_SECS)
            return True

    async def record(
        self,
        *,
        client: str,
        provider: str,
        model: str,
        prompt: int,
        completion: int,
        usage_source: str = "upstream",
    ) -> None:
        """Record one admitted request (R03: always count, even on zero usage).

        usage_source is "upstream" when the provider reported usage and
        "estimated" when tokens were heuristically estimated from text.
        """
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
                    "usage_source": usage_source,
                },
            )
        )


def estimate_usage(prompt_text: str, completion_text: str) -> tuple[int, int]:
    """Heuristic token estimate (~4 chars per token) used when the provider
    sends no usage object (R03). Explicitly marked, never silently zero."""
    return (len(prompt_text) // 4, len(completion_text) // 4)


# R05: atomic admission Lua script.
#
# KEYS: c_tokens, c_req, p_tokens, p_req, m_tokens, rsv_key
# ARGV: c_token_limit, c_req_limit, p_token_limit (-1 = no limit),
#       reserve_total, rsv_value, rsv_ttl_secs, bucket_ttl_secs
#
# Atomically checks request + token budgets and reserves
# (prompt estimate + completion cap) on client and provider counters.
# Returns {ok, reserved} or {scope, used} on quota breach.
#
# Supported Redis modes: standalone + Sentinel (single primary). Redis
# Cluster is explicitly unsupported: the script touches keys in different
# hash slots and Cluster rejects cross-slot scripts.
ADMIT_LUA = """
local ct = tonumber(redis.call('GET', KEYS[1]) or '0')
local cr = tonumber(redis.call('GET', KEYS[2]) or '0')
local pt = tonumber(redis.call('GET', KEYS[3]) or '0')
local reserve = tonumber(ARGV[4])
if tonumber(ARGV[1]) >= 0 and ct + reserve > tonumber(ARGV[1]) then
  return {'tokens', ct}
end
if tonumber(ARGV[2]) >= 0 and cr + 1 > tonumber(ARGV[2]) then
  return {'requests', cr}
end
if tonumber(ARGV[3]) >= 0 and pt + reserve > tonumber(ARGV[3]) then
  return {'provider_tokens', pt}
end
redis.call('INCRBY', KEYS[1], reserve)
redis.call('INCR', KEYS[2])
redis.call('INCRBY', KEYS[3], reserve)
redis.call('INCR', KEYS[4])
redis.call('INCRBY', KEYS[5], reserve)
for i = 1, 5 do redis.call('EXPIRE', KEYS[i], ARGV[7]) end
redis.call('SET', KEYS[6], ARGV[5], 'EX', ARGV[6])
return {'ok', reserve}
"""

# R05: idempotent settle Lua script.
#
# KEYS: rsv_key, c_tokens, p_tokens, m_tokens
# ARGV: delta (actual - reserved, may be negative), bucket_ttl_secs
#
# Applies only when the reservation is still open; repeat or post-expiry
# settles are no-ops. Crash policy is conservative: an unsettled
# reservation keeps its budget consumed (never auto-refunded).
SETTLE_LUA = """
if redis.call('GET', KEYS[1]) ~= 'open' then return 0 end
redis.call('SET', KEYS[1], 'settled', 'KEEPTTL')
local d = tonumber(ARGV[1])
for i = 2, 4 do
  local v = tonumber(redis.call('GET', KEYS[i]) or '0') + d
  if v < 0 then v = 0 end
  redis.call('SET', KEYS[i], v, 'EX', ARGV[2])
end
return 1
"""

_RSV_TTL_SECS = 600
_BUCKET_TTL_SECS = 60 * 60 * 48


@dataclass(frozen=True)
class Reservation:
    """Open budget reservation from Metering.admit (R05)."""

    id: str
    client: str
    provider: str
    model: str
    prompt_est: int
    completion_cap: int

    @property
    def reserved_total(self) -> int:
        return self.prompt_est + self.completion_cap

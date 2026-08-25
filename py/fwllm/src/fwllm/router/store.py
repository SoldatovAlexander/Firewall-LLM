"""State stores for the policy engine.

InMemoryRouterStore is the default (state lost on restart).
RedisRouterStore persists blocked sources, provider override and token
counters across gateway restarts.
"""

from __future__ import annotations

from typing import Any, Protocol


class RouterStateStore(Protocol):
    def get_override(self) -> tuple[str, float] | None: ...
    def set_override(self, provider: str, until_ts: float) -> None: ...
    def clear_override(self) -> None: ...
    def block_source(self, client: str, until_ts: float) -> None: ...
    def is_blocked(self, client: str, now_ts: float) -> bool: ...
    def incr_tokens(self, key: tuple[str, str], amount: int) -> int: ...
    def get_tokens(self, key: tuple[str, str]) -> int: ...


class InMemoryRouterStore:
    def __init__(self) -> None:
        self._override: tuple[str, float] | None = None
        self._blocked: dict[str, float] = {}
        self._tokens: dict[tuple[str, str], int] = {}

    def get_override(self) -> tuple[str, float] | None:
        return self._override

    def set_override(self, provider: str, until_ts: float) -> None:
        self._override = (provider, until_ts)

    def clear_override(self) -> None:
        self._override = None

    def block_source(self, client: str, until_ts: float) -> None:
        self._blocked[client] = until_ts

    def is_blocked(self, client: str, now_ts: float) -> bool:
        return self._blocked.get(client, 0) > now_ts

    def incr_tokens(self, key: tuple[str, str], amount: int) -> int:
        self._tokens[key] = self._tokens.get(key, 0) + amount
        return self._tokens[key]

    def get_tokens(self, key: tuple[str, str]) -> int:
        return self._tokens.get(key, 0)


class RedisRouterStore:
    PREFIX = "fwllm:rt"

    def __init__(self, client: Any):
        self._client = client

    def _tokens_key(self, key: tuple[str, str]) -> str:
        provider, day = key
        return f"{self.PREFIX}:tokens:{provider}:{day}"

    def get_override(self) -> tuple[str, float] | None:
        provider = self._client.get(f"{self.PREFIX}:override")
        until = self._client.get(f"{self.PREFIX}:override_until")
        if provider and until:
            return provider.decode() if isinstance(provider, bytes) else provider, float(until)
        return None

    def set_override(self, provider: str, until_ts: float) -> None:
        self._client.set(f"{self.PREFIX}:override", provider)
        self._client.set(f"{self.PREFIX}:override_until", until_ts)

    def clear_override(self) -> None:
        self._client.delete(
            f"{self.PREFIX}:override", f"{self.PREFIX}:override_until"
        )

    def block_source(self, client: str, until_ts: float) -> None:
        self._client.set(f"{self.PREFIX}:blocked:{client}", until_ts)

    def is_blocked(self, client: str, now_ts: float) -> bool:
        raw = self._client.get(f"{self.PREFIX}:blocked:{client}")
        if raw is None:
            return False
        until = float(raw)
        if until <= now_ts:
            self._client.delete(f"{self.PREFIX}:blocked:{client}")
            return False
        return True

    def incr_tokens(self, key: tuple[str, str], amount: int) -> int:
        value: int = self._client.incrby(self._tokens_key(key), amount)
        self._client.expire(self._tokens_key(key), 60 * 60 * 48)
        return value

    def get_tokens(self, key: tuple[str, str]) -> int:
        raw = self._client.get(self._tokens_key(key))
        return int(raw) if raw else 0

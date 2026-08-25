"""Enterprise egress: proxy pools with rotation (commercial module).

Requires a commercial license. See README.md in this package.
"""

from __future__ import annotations

from collections.abc import Callable
from datetime import UTC, datetime

from fwllm.config import PoolConfig


def _default_clock() -> datetime:
    return datetime.now(UTC)


class ProxyPool:
    """Rotating pool of egress proxies for one adapter binding."""

    def __init__(
        self,
        *,
        proxies: list[str],
        rotation: str = "round_robin",
        requests_per_proxy: int = 100,
        fail_threshold: int = 3,
        cooldown_seconds: int = 300,
        clock: Callable[[], datetime] | None = None,
    ):
        if not proxies:
            raise ValueError("ProxyPool requires at least one proxy")
        self._proxies = list(proxies)
        self._rotation = rotation
        self._requests_per_proxy = requests_per_proxy
        self._fail_threshold = fail_threshold
        self._cooldown_seconds = cooldown_seconds
        self._clock = clock or _default_clock

        self._index = 0
        self._window_count = 0
        self._failures: dict[str, int] = {}
        self._disabled_until: dict[str, float] = {}

    # -- availability ---------------------------------------------------------

    def _now_ts(self) -> float:
        return self._clock().timestamp()

    def _available(self) -> list[str]:
        now = self._now_ts()
        available: list[str] = []
        for url in self._proxies:
            until = self._disabled_until.get(url, 0)
            if until and until > now:
                continue
            # cooldown elapsed -> clear failure state lazily
            if url in self._disabled_until:
                del self._disabled_until[url]
                self._failures.pop(url, None)
            available.append(url)
        return available or list(self._proxies)

    def current(self) -> str:
        available = self._available()
        if len(available) == 1:
            url = available[0]
        elif self._rotation == "least_used":
            url = min(available, key=lambda u: u == self._active_url())
        else:
            url = available[self._index % len(available)]
        return url

    def _active_url(self) -> str:
        return self._proxies[self._index % len(self._proxies)]

    def reset_window(self) -> None:
        self._window_count = 0

    def mark_request(self) -> None:
        self._window_count += 1
        if self._window_count >= self._requests_per_proxy:
            self._advance()

    def _advance(self) -> None:
        self._index += 1
        self.reset_window()

    def report_failure(self, url: str) -> None:
        failures = self._failures.get(url, 0) + 1
        self._failures[url] = failures
        if failures >= self._fail_threshold:
            self._disabled_until[url] = self._now_ts() + self._cooldown_seconds


def build_pool(config: PoolConfig, clock: Callable[[], datetime] | None = None) -> ProxyPool:
    return ProxyPool(
        proxies=list(config.proxies),
        rotation=config.rotation,
        requests_per_proxy=config.requests_per_proxy,
        fail_threshold=config.fail_threshold,
        cooldown_seconds=config.cooldown_seconds,
        clock=clock,
    )

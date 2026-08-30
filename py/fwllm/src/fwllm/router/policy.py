"""Policy engine: provider selection, budget rules, attack failover."""

from __future__ import annotations

import logging
from collections.abc import Callable
from datetime import UTC, datetime
from typing import TypeVar

from fwllm.config import AttackFailoverConfig, ConfigError, RoutingConfig, RoutingRule
from fwllm.metering import Event
from fwllm.providers.base import BlockedError
from fwllm.router.store import InMemoryRouterStore, RouterStateStore

logger = logging.getLogger(__name__)

_SEVERITY: dict[str, int] = {"low": 0, "medium": 1, "high": 2, "critical": 3}

_T = TypeVar("_T")


def _default_clock() -> datetime:
    return datetime.now(UTC)


class PolicyEngine:
    """Resolves which provider serves a request and under which model name.

    Event-driven: consumes tokens_spent / attack_detected / quota_exceeded
    events. State is in-memory for MVP; Redis persistence is a follow-up.
    """

    def __init__(
        self,
        routing: RoutingConfig,
        *,
        clock: Callable[[], datetime] | None = None,
        store: RouterStateStore | None = None,
    ):
        self._routing = routing
        self._clock = clock or _default_clock
        self._store = store or InMemoryRouterStore()
        self._attack_times: list[tuple[str, datetime]] = []

    @staticmethod
    def validate_routing(routing: RoutingConfig, known_providers: list[str]) -> None:
        for name in routing.default_chain:
            if name not in known_providers:
                raise ConfigError(
                    f"routing.default_chain references unknown provider '{name}'"
                )
        switch_to = routing.attack_failover.switch_to
        if switch_to and switch_to not in known_providers:
            raise ConfigError(
                f"attack_failover.switch_to references unknown provider '{switch_to}'"
            )

    def _day(self) -> str:
        return self._clock().strftime("%Y%m%d")

    def _now_ts(self) -> float:
        return self._clock().timestamp()

    def _safe(self, fn: Callable[[], _T], default: _T) -> _T:
        """State store outages must not break traffic (fail-open)."""
        try:
            return fn()
        except Exception:  # noqa: BLE001
            logger.warning("router state store unavailable, failing open")
            return default

    def provider_tokens_today(self, provider: str) -> int:
        return self._safe(
            lambda: self._store.get_tokens((provider, self._day())), 0
        )

    def on_event(self, event: Event) -> None:
        if event.name == "tokens_spent":
            provider = str(event.data.get("provider", ""))
            amount = int(event.data.get("total_tokens", 0))
            day = self._day()
            self._safe(lambda: self._store.incr_tokens((provider, day), amount), None)
            return
        af: AttackFailoverConfig = self._routing.attack_failover
        if not af.enabled or event.name != "attack_detected":
            return
        severity = str(event.data.get("severity", "low"))
        if _SEVERITY[severity] < _SEVERITY[af.min_severity]:
            return
        now = self._clock()
        window_start = now.timestamp() - af.window_seconds
        source = str(event.data.get("client", ""))
        self._attack_times = [
            pair for pair in self._attack_times if pair[1].timestamp() >= window_start
        ]
        self._attack_times.append((source, now))
        if len(self._attack_times) < af.count:
            return
        del self._attack_times[:]  # window consumed - one reaction per burst
        if af.block_source and source:
            until = now.timestamp() + af.block_ttl_seconds
            self._safe(lambda: self._store.block_source(source, until), None)
        if af.switch_to:
            cooldown_until = now.timestamp() + af.cooldown_seconds
            switch_provider = af.switch_to
            self._safe(
                lambda: self._store.set_override(switch_provider, cooldown_until), None
            )

    def _chain_candidates(self) -> list[str]:
        chain = list(self._routing.default_chain) or ["default"]
        override = self._safe(self._store.get_override, None)
        if override is None:
            return chain
        provider, until_ts = override
        if self._now_ts() < until_ts:
            return [provider] + [p for p in chain if p != provider]
        self._safe(self._store.clear_override, None)  # cooldown expired
        return chain

    def _violates_rule(self, provider: str, rule: RoutingRule) -> bool:
        cond = rule.when
        if cond.provider and cond.provider != provider:
            return False
        if cond.provider_tokens_today is not None and not cond.provider_tokens_today.matches(
            float(self.provider_tokens_today(provider))
        ):
            return False
        return True

    def resolve(self, requested_model: str, client_id: str) -> tuple[str, str]:
        from fwllm.metering import QuotaExceeded

        now_ts = self._now_ts()
        if self._safe(lambda: self._store.is_blocked(client_id, now_ts), False):
            raise BlockedError(
                "request source is temporarily blocked", reason="blocked_source"
            )

        candidates = self._chain_candidates()
        mapping = self._routing.model_mapping.get(requested_model, {})
        # Check each candidate against rules, handling action
        for idx, candidate in enumerate(candidates):
            violating = [r for r in self._routing.rules if self._violates_rule(candidate, r)]
            if not violating:
                return candidate, mapping.get(candidate, requested_model)
            # Handle first violating rule's action
            rule = violating[0]
            if rule.action.switch_to:
                switch_to = rule.action.switch_to
                # Validate switch_to is known
                if switch_to in candidates:
                    # Move switch_to to front and re-evaluate
                    continue
                return switch_to, mapping.get(switch_to, requested_model)
            if rule.action.next_in_chain:
                # Skip to next candidate
                continue
        # All candidates exhausted
        raise QuotaExceeded(
            f"all providers in chain {candidates} exceeded budget/rules",
            limit=0,
            scope="provider_budget",
        )

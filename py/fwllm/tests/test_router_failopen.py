"""Router must degrade gracefully when its state store is unavailable."""

from datetime import UTC, datetime

from fwllm.config import RoutingConfig
from fwllm.metering import Event
from fwllm.router.policy import PolicyEngine


class BrokenStore:
    """Simulates redis being down."""

    def get_override(self) -> tuple[str, float] | None:
        raise ConnectionError("redis down")

    def set_override(self, provider: str, until_ts: float) -> None:
        raise ConnectionError("redis down")

    def clear_override(self) -> None:
        raise ConnectionError("redis down")

    def block_source(self, client: str, until_ts: float) -> None:
        raise ConnectionError("redis down")

    def is_blocked(self, client: str, now_ts: float) -> bool:
        raise ConnectionError("redis down")

    def incr_tokens(self, key: tuple[str, str], amount: int) -> int:
        raise ConnectionError("redis down")

    def get_tokens(self, key: tuple[str, str]) -> int:
        raise ConnectionError("redis down")


def _clock() -> datetime:
    return datetime(2026, 8, 25, 12, 0, tzinfo=UTC)


def test_events_do_not_raise_when_store_down():
    engine = PolicyEngine(
        RoutingConfig(default_chain=["p"]), store=BrokenStore(), clock=_clock  # type: ignore[arg-type]
    )
    engine.on_event(Event("tokens_spent", {"provider": "p", "total_tokens": 5}))
    engine.on_event(Event("attack_detected", {"severity": "critical", "client": "x"}))


def test_resolve_fails_open_when_store_down():
    engine = PolicyEngine(
        RoutingConfig(default_chain=["p"]), store=BrokenStore(), clock=_clock  # type: ignore[arg-type]
    )
    assert engine.resolve("m", "alice") == ("p", "m")


def test_budget_rules_skipped_when_store_down():
    from fwllm.config import RoutingRule, RuleAction, RuleCondition

    routing = RoutingConfig(
        default_chain=["primary", "backup"],
        rules=[
            RoutingRule(
                name="budget",
                when=RuleCondition(provider="primary", provider_tokens_today={"gt": 1}),
                action=RuleAction(next_in_chain=True),
            )
        ],
    )
    engine = PolicyEngine(routing, store=BrokenStore(), clock=_clock)  # type: ignore[arg-type]
    # cannot read counters -> cannot prove violation -> keep chain head
    assert engine.resolve("m", "alice") == ("primary", "m")

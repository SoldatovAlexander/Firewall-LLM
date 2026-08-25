"""Router + Policy Engine tests."""

from datetime import UTC, datetime
from typing import Any

import pytest

from fwllm.config import (
    AttackFailoverConfig,
    ConfigError,
    RoutingConfig,
    RoutingRule,
    RuleAction,
    RuleCondition,
)
from fwllm.metering import Event
from fwllm.providers.base import BlockedError
from fwllm.router.policy import PolicyEngine


def _clock(day: int = 25, minute: int = 0):
    base = datetime(2026, 8, day, 12, minute, tzinfo=UTC)
    return lambda: base


def _engine(routing: RoutingConfig, clock: Any = None) -> PolicyEngine:
    return PolicyEngine(routing=routing, clock=clock or _clock())


# --- basic resolution ---------------------------------------------------------


def test_resolve_uses_default_chain_order():
    engine = _engine(RoutingConfig(default_chain=["openrouter", "local"]))
    assert engine.resolve("gpt-4o", "alice") == ("openrouter", "gpt-4o")


def test_model_mapping_translates_concrete_model():
    engine = _engine(
        RoutingConfig(
            default_chain=["local"],
            model_mapping={"gpt-4o": {"local": "llama3.3:70b"}},
        )
    )
    assert engine.resolve("gpt-4o", "alice") == ("local", "llama3.3:70b")


def test_unknown_model_passes_through():
    engine = _engine(RoutingConfig(default_chain=["openrouter"]))
    assert engine.resolve("weird-model", "alice") == ("openrouter", "weird-model")


# --- token budget rule ----------------------------------------------------------


def _tokens_event(provider: str, total: int) -> Event:
    return Event("tokens_spent", {"provider": provider, "total_tokens": total})


def test_token_budget_rule_switches_provider():
    engine = _engine(
        RoutingConfig(
            default_chain=["primary", "backup"],
            rules=[
                RoutingRule(
                    name="budget",
                    when=RuleCondition(
                        provider="primary",
                        provider_tokens_today={"gt": 50},
                    ),
                    action=RuleAction(next_in_chain=True),
                )
            ],
        )
    )
    engine.on_event(_tokens_event("primary", 100))
    assert engine.resolve("m", "alice") == ("backup", "m")


def test_under_budget_keeps_primary():
    engine = _engine(
        RoutingConfig(
            default_chain=["primary", "backup"],
            rules=[
                RoutingRule(
                    name="budget",
                    when=RuleCondition(
                        provider="primary", provider_tokens_today={"gt": 50}
                    ),
                    action=RuleAction(next_in_chain=True),
                )
            ],
        )
    )
    engine.on_event(_tokens_event("primary", 10))
    assert engine.resolve("m", "alice") == ("primary", "m")


# --- attack failover ------------------------------------------------------------


def test_attack_events_block_source_and_switch_default():
    routing = RoutingConfig(
        default_chain=["cloud"],
        attack_failover=AttackFailoverConfig(
            enabled=True,
            count=3,
            window_seconds=300,
            min_severity="high",
            switch_to="local",
            block_source=True,
            block_ttl_seconds=600,
        ),
    )
    engine = _engine(routing)
    for _ in range(3):
        engine.on_event(
            Event(
                "attack_detected",
                {"severity": "high", "kind": "prompt_injection", "client": "attacker"},
            )
        )
    with pytest.raises(BlockedError):
        engine.resolve("m", "attacker")
    assert engine.resolve("m", "victim") == ("local", "m")


def test_attack_failover_cooldown_expires():
    now = {"t": datetime(2026, 8, 25, 12, 0, tzinfo=UTC)}

    def clock() -> datetime:
        return now["t"]

    routing = RoutingConfig(
        default_chain=["cloud"],
        attack_failover=AttackFailoverConfig(
            enabled=True,
            count=1,
            window_seconds=60,
            min_severity="high",
            switch_to="local",
            block_source=False,
            cooldown_seconds=120,
        ),
    )
    engine = _engine(routing, clock=clock)
    engine.on_event(Event("attack_detected", {"severity": "critical"}))
    assert engine.resolve("m", "bob") == ("local", "m")
    now["t"] = datetime(2026, 8, 25, 12, 5, tzinfo=UTC)
    assert engine.resolve("m", "bob") == ("cloud", "m")


def test_disabled_attack_failover_ignores_events():
    engine = _engine(
        RoutingConfig(
            default_chain=["cloud"],
            attack_failover=AttackFailoverConfig(enabled=False),
        )
    )
    for _ in range(10):
        engine.on_event(Event("attack_detected", {"severity": "critical"}))
    assert engine.resolve("m", "attacker") == ("cloud", "m")


# --- config validation -----------------------------------------------------------


def test_chain_must_reference_known_providers():
    cfg_like_providers = ["p1"]
    routing = RoutingConfig(default_chain=["ghost"])
    with pytest.raises(ConfigError, match="ghost"):
        PolicyEngine.validate_routing(routing, cfg_like_providers)


# --- helpers ---------------------------------------------------------------------


def test_routed_from_flag_needed_when_remapped():
    engine = _engine(
        RoutingConfig(
            default_chain=["local"],
            model_mapping={"gpt-4o": {"local": "llama3.3:70b"}},
        )
    )
    provider, model = engine.resolve("gpt-4o", "alice")
    assert model != "gpt-4o"

"""Router state persistence: blocked sources, override, token counters."""

from datetime import UTC, datetime

import fakeredis
import pytest

from fwllm.config import (
    AttackFailoverConfig,
    RoutingConfig,
    RoutingRule,
    RuleAction,
    RuleCondition,
)
from fwllm.metering import Event
from fwllm.providers.base import BlockedError
from fwllm.router.policy import PolicyEngine
from fwllm.router.store import InMemoryRouterStore, RedisRouterStore


def _clock(day: int = 25, minute: int = 0):
    return lambda: datetime(2026, 8, day, 12, minute, tzinfo=UTC)


def _redis_store() -> RedisRouterStore:
    return RedisRouterStore(fakeredis.FakeStrictRedis(decode_responses=True))


def _attack_event(client: str = "attacker") -> Event:
    return Event(
        "attack_detected",
        {"severity": "high", "kind": "prompt_injection", "client": client},
    )


def _attack_routing(**kwargs) -> RoutingConfig:
    return RoutingConfig(
        default_chain=["cloud"],
        attack_failover=AttackFailoverConfig(
            enabled=True,
            count=2,
            window_seconds=300,
            min_severity="high",
            switch_to="local",
            block_source=True,
            **kwargs,
        ),
    )


def _budget_routing() -> RoutingConfig:
    return RoutingConfig(
        default_chain=["primary", "backup"],
        rules=[
            RoutingRule(
                name="budget",
                when=RuleCondition(provider="primary", provider_tokens_today={"gt": 50}),
                action=RuleAction(next_in_chain=True),
            )
        ],
    )


def test_in_memory_store_roundtrip():
    store = InMemoryRouterStore()
    store.set_override("local", until_ts=99999)
    assert store.get_override() == ("local", 99999.0)
    store.block_source("alice", until_ts=99999)
    assert store.is_blocked("alice", now_ts=1)
    assert not store.is_blocked("alice", now_ts=200000)
    store.incr_tokens(("primary", "20260825"), 40)
    store.incr_tokens(("primary", "20260825"), 15)
    assert store.get_tokens(("primary", "20260825")) == 55


def test_redis_store_survives_engine_restart():
    store = _redis_store()
    engine_a = PolicyEngine(_attack_routing(), store=store, clock=_clock())
    engine_a.on_event(_attack_event())
    engine_a.on_event(_attack_event())

    # "restart": brand-new engine over the same redis
    engine_b = PolicyEngine(_attack_routing(), store=store, clock=_clock())
    with pytest.raises(BlockedError):
        engine_b.resolve("m", "attacker")
    assert engine_b.resolve("m", "victim") == ("local", "m")


def test_redis_blocked_source_expires():
    store = _redis_store()
    engine_a = PolicyEngine(_attack_routing(block_ttl_seconds=100), store=store, clock=_clock())
    engine_a.on_event(_attack_event())
    engine_a.on_event(_attack_event())

    # after block_ttl the source is unblocked (clock moved past until_ts);
    # the provider override also cooled down, so routing returns to the chain
    engine_b = PolicyEngine(
        _attack_routing(block_ttl_seconds=100),
        store=store,
        clock=_clock(minute=5),
    )
    assert engine_b.resolve("m", "attacker") == ("cloud", "m")


def test_redis_token_counters_survive_restart():
    store = _redis_store()
    engine_a = PolicyEngine(_budget_routing(), store=store, clock=_clock())
    engine_a.on_event(Event("tokens_spent", {"provider": "primary", "total_tokens": 100}))

    engine_b = PolicyEngine(_budget_routing(), store=store, clock=_clock())
    assert engine_b.resolve("m", "alice") == ("backup", "m")


def test_default_store_is_in_memory():
    engine = PolicyEngine(RoutingConfig(default_chain=["x"]), clock=_clock())
    assert isinstance(engine._store, InMemoryRouterStore)

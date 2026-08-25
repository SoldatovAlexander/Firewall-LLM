"""Enterprise proxy pool tests (commercial module)."""

from datetime import UTC, datetime

import pydantic
import pytest
from fwllm_enterprise.egress_pools import ProxyPool

from fwllm.config import ConfigError, EgressConfig, PoolConfig


def _clock(minute: int = 0):
    return lambda: datetime(2026, 8, 25, 12, minute, tzinfo=UTC)


def test_round_robin_rotates_after_n_requests():
    pool = ProxyPool(
        proxies=["http://p1:8080", "http://p2:8080"],
        rotation="round_robin",
        requests_per_proxy=3,
    )
    for _ in range(7):
        pool.current()
    for _ in range(6):
        pool.mark_request()
    # first 3 stay on p1, then rotate to p2
    assert pool.current() == "http://p1:8080"
    pool.mark_request()
    pool.mark_request()
    pool.mark_request()
    assert pool.current() == "http://p2:8080"


def test_single_proxy_pool_stays_put():
    pool = ProxyPool(proxies=["http://only:8080"], requests_per_proxy=5)
    for _ in range(10):
        pool.mark_request()
    assert pool.current() == "http://only:8080"


def test_failures_disable_proxy_until_cooldown():
    pool = ProxyPool(
        proxies=["http://a:8080", "http://b:8080"],
        rotation="round_robin",
        fail_threshold=2,
        cooldown_seconds=300,
        clock=_clock(),
    )
    pool.report_failure("http://a:8080")
    pool.report_failure("http://a:8080")
    # 'a' disabled -> traffic shifts to 'b' even though rotation window not over
    pool.reset_window()
    assert pool.current() == "http://b:8080"

    recovered = ProxyPool(
        proxies=["http://a:8080", "http://b:8080"],
        fail_threshold=2,
        cooldown_seconds=300,
        clock=_clock(minute=6),
    )
    recovered.report_failure("http://a:8080")
    recovered.report_failure("http://a:8080")
    recovered.reset_window()
    # cooldown expired relative to fresh clock -> 'a' available again
    assert recovered.current() in ("http://a:8080", "http://b:8080")


def test_all_proxies_disabled_falls_back_to_first():
    pool = ProxyPool(
        proxies=["http://a:8080", "http://b:8080"],
        fail_threshold=1,
        cooldown_seconds=300,
        clock=_clock(),
    )
    pool.report_failure("http://a:8080")
    pool.report_failure("http://b:8080")
    pool.reset_window()
    assert pool.current() == "http://a:8080"


def test_least_used_strategy():
    pool = ProxyPool(
        proxies=["http://a:8080", "http://b:8080"],
        rotation="least_used",
        requests_per_proxy=100,
    )
    pool.mark_request()
    pool.mark_request()
    pool.mark_request()
    # all requests counted against active 'a'; least_used switches to 'b'
    assert pool.current() == "http://b:8080"


# --- config -------------------------------------------------------------------


def test_pools_mode_requires_bindings_and_pools():
    with pytest.raises((ConfigError, pydantic.ValidationError), match="pools requires"):
        EgressConfig(mode="pools")
    with pytest.raises((ConfigError, pydantic.ValidationError), match="binding"):
        EgressConfig(
            mode="pools",
            pools={"main": PoolConfig(proxies=["http://p:8080"])},
            bindings={"adapter": "nonexistent"},
        )


def test_binding_to_unknown_pool_rejected():
    with pytest.raises(
        (ConfigError, pydantic.ValidationError), match="unknown pool 'missing'"
    ):
        EgressConfig(
            mode="pools",
            pools={"main": PoolConfig(proxies=["http://p:8080"])},
            bindings={"adapter": "missing"},
        )

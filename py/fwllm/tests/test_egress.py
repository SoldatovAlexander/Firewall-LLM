"""Egress tests: MVP supports exactly two modes - direct or one global proxy."""

import pydantic
import pytest

from fwllm.config import Config, ConfigError, EgressConfig, ProviderConfig
from fwllm.egress import build_http_client
from fwllm.providers.registry import build_providers


class _CaptureClient:
    def __init__(self, **kwargs):
        self.kwargs = kwargs


def test_default_is_direct(monkeypatch):
    import fwllm.egress as egress

    captured = {}
    monkeypatch.setattr(
        egress.httpx, "AsyncClient", lambda **kw: captured.update(kw) or _CaptureClient()
    )
    build_http_client("http://api.test/v1", None)
    assert "proxy" not in captured


def test_single_proxy_mode_passes_proxy(monkeypatch):
    import fwllm.egress as egress

    captured = {}
    monkeypatch.setattr(
        egress.httpx, "AsyncClient", lambda **kw: captured.update(kw) or _CaptureClient()
    )
    build_http_client("http://api.test/v1", "http://proxy.corp:8080")
    assert captured["proxy"] == "http://proxy.corp:8080"


def test_config_defaults_to_direct():
    cfg = Config(providers={"p": ProviderConfig(base_url="https://x/v1")})
    assert cfg.egress.mode == "direct"
    assert cfg.egress.proxy_url is None


def test_single_proxy_requires_proxy_url():
    # raised as pydantic error on direct construction, wrapped into
    # ConfigError by load_config for file-based configuration
    with pytest.raises((ConfigError, pydantic.ValidationError), match="proxy_url"):
        EgressConfig(mode="single_proxy")


def test_single_proxy_with_url_valid():
    cfg_egress = EgressConfig(mode="single_proxy", proxy_url="socks5://p:1080")
    assert cfg_egress.proxy_url == "socks5://p:1080"


def test_registry_applies_proxy_to_all_adapters(monkeypatch):
    import fwllm.egress as egress

    captured = []
    monkeypatch.setattr(
        egress.httpx,
        "AsyncClient",
        lambda **kw: captured.append(kw) or _CaptureClient(),
    )
    cfg = Config(
        providers={
            "a": ProviderConfig(type="openrouter", base_url="https://a/v1"),
            "b": ProviderConfig(type="ollama", base_url="http://b/v1"),
        },
        egress=EgressConfig(mode="single_proxy", proxy_url="http://one.proxy:3128"),
    )
    build_providers(cfg)
    assert len(captured) == 2
    assert all(kw["proxy"] == "http://one.proxy:3128" for kw in captured)


def test_registry_direct_has_no_proxy_key(monkeypatch):
    import fwllm.egress as egress

    captured = []
    monkeypatch.setattr(
        egress.httpx,
        "AsyncClient",
        lambda **kw: captured.append(kw) or _CaptureClient(),
    )
    cfg = Config(
        providers={"a": ProviderConfig(base_url="https://a/v1")},
        egress=EgressConfig(),
    )
    build_providers(cfg)
    assert captured and "proxy" not in captured[0]

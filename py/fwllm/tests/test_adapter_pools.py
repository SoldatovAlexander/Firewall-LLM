"""Adapter + enterprise pool integration tests."""

from typing import Any

import httpx
import pytest
import respx

import fwllm.adapters.openai_compat as compat_module
from fwllm.adapters.openai_compat import OpenAICompatAdapter
from fwllm.providers.base import ProviderError

BASE = "https://api.test/v1"

RESPONSE = {
    "id": "x",
    "object": "chat.completion",
    "created": 1,
    "model": "m",
    "choices": [{"index": 0, "message": {"role": "assistant", "content": "ok"}}],
}

PAYLOAD = {"model": "m", "messages": [{"role": "user", "content": "hi"}]}


class FakeManager:
    def __init__(self, proxies: list[str | None]):
        self._proxies = list(proxies)
        self.marks = 0
        self.failures: list[str] = []

    def current(self) -> str | None:
        return self._proxies[0] if len(self._proxies) == 1 else self._proxies.pop(0)

    def mark(self) -> None:
        self.marks += 1

    def report_failure(self, url: str) -> None:
        self.failures.append(url)


@respx.mock
async def test_proxy_rotation_rebuilds_client(monkeypatch):
    built: list[dict[str, Any]] = []

    class RecordingClient(httpx.AsyncClient):
        def __init__(self, **kwargs):  # type: ignore[no-untyped-def]
            built.append(kwargs)
            super().__init__(**kwargs)

    monkeypatch.setattr(compat_module.httpx, "AsyncClient", RecordingClient)

    manager = FakeManager(["http://p1:8080", "http://p2:8080"])
    adapter = OpenAICompatAdapter(
        api_key="k", base_url=BASE, timeout=30, proxy_manager=manager
    )

    respx.post(f"{BASE}/chat/completions").mock(
        side_effect=[
            httpx.Response(200, json=RESPONSE),
            httpx.Response(200, json=RESPONSE),
        ]
    )
    await adapter.chat(PAYLOAD)
    await adapter.chat(PAYLOAD)

    assert [b.get("proxy") for b in built] == ["http://p1:8080", "http://p2:8080"]
    assert all(b["base_url"] == BASE for b in built)
    assert manager.marks == 2


@respx.mock
async def test_connection_error_reports_failure_to_pool():
    manager = FakeManager(["http://p1:8080"])
    adapter = OpenAICompatAdapter(
        api_key="k", base_url=BASE, proxy_manager=manager
    )
    respx.post(f"{BASE}/chat/completions").mock(
        side_effect=httpx.ConnectError("down")
    )
    with pytest.raises(ProviderError):
        await adapter.chat(PAYLOAD)
    assert manager.failures == ["http://p1:8080"]


def test_adapter_requires_client_or_manager():
    with pytest.raises(ValueError, match="client"):
        OpenAICompatAdapter(api_key="k")


def test_registry_pools_mode_uses_enterprise_package():
    from fwllm.config import Config, EgressConfig, PoolConfig, ProviderConfig
    from fwllm.providers.registry import build_providers

    cfg = Config(
        providers={"a": ProviderConfig(base_url="https://a/v1")},
        egress=EgressConfig(
            mode="pools",
            pools={
                "main": PoolConfig(
                    proxies=["http://p1:8080", "http://p2:8080"],
                    requests_per_proxy=2,
                )
            },
            bindings={"a": "main"},
        ),
    )
    providers = build_providers(cfg)
    # first proxy handed out by the pool
    assert providers["a"]._ensure_client() is not None  # noqa: SLF001

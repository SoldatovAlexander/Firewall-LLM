"""Shared contract tests for every provider adapter.

One suite, run against all adapters: they must behave identically towards
the gateway regardless of the external API behind them.
"""

from collections.abc import AsyncIterator
from typing import Any

import httpx
import pytest
import respx

from fwllm.adapters.ollama import OllamaAdapter
from fwllm.adapters.openai_compat import OpenAICompatAdapter
from fwllm.adapters.openrouter import OpenRouterAdapter
from fwllm.providers.base import ProviderError

BASE = "https://api.test/v1"

COMPLETION_RESPONSE: dict[str, Any] = {
    "id": "chatcmpl-x",
    "object": "chat.completion",
    "created": 1700000000,
    "model": "some/model",
    "choices": [
        {
            "index": 0,
            "message": {"role": "assistant", "content": "ok"},
            "finish_reason": "stop",
        }
    ],
    "usage": {"prompt_tokens": 4, "completion_tokens": 1, "total_tokens": 5},
}

SSE_BODY = (
    'data: {"id":"c","object":"chat.completion.chunk","created":1,'
    '"model":"m","choices":[{"index":0,"delta":{"content":"He"},"finish_reason":null}]}\n'
    "\n"
    'data: {"id":"c","object":"chat.completion.chunk","created":1,'
    '"model":"m","choices":[{"index":0,"delta":{"content":"y"},"finish_reason":"stop"}]}\n'
    "\n"
    "data: [DONE]\n"
    "\n"
)

PAYLOAD: dict[str, Any] = {
    "model": "some/model",
    "messages": [{"role": "user", "content": "hi"}],
    "stream": False,
}


def _make_adapters() -> list[tuple[str, Any]]:
    def build(cls: type, **kwargs: Any) -> Any:
        client = httpx.AsyncClient(base_url=BASE)
        return cls(client=client, api_key="sk-test", **kwargs)

    return [
        ("openai_compat", build(OpenAICompatAdapter)),
        ("openrouter", build(OpenRouterAdapter)),
        ("ollama", build(OllamaAdapter)),
    ]


ADAPTER_IDS = ["openai_compat", "openrouter", "ollama"]


@pytest.fixture(params=_make_adapters(), ids=ADAPTER_IDS)
def adapter(request: pytest.FixtureRequest) -> Any:
    return request.param[1]


@respx.mock
async def test_chat_posts_to_completions_and_returns_json(adapter: Any) -> None:
    route = respx.post(f"{BASE}/chat/completions").respond(json=COMPLETION_RESPONSE)
    result = await adapter.chat(PAYLOAD)
    assert route.called
    assert result["usage"]["total_tokens"] == 5
    assert result["choices"][0]["message"]["content"] == "ok"


@respx.mock
async def test_chat_sends_bearer_auth(adapter: Any) -> None:
    route = respx.post(f"{BASE}/chat/completions").respond(json=COMPLETION_RESPONSE)
    await adapter.chat(PAYLOAD)
    request = route.calls.last.request
    assert request.headers["authorization"] == "Bearer sk-test"


@respx.mock
async def test_provider_error_on_http_status(adapter: Any) -> None:
    respx.post(f"{BASE}/chat/completions").respond(status_code=429, json={"error": {}})
    with pytest.raises(ProviderError) as exc_info:
        await adapter.chat(PAYLOAD)
    assert exc_info.value.status == 429


@respx.mock
async def test_network_error_maps_to_provider_error(adapter: Any) -> None:
    respx.post(f"{BASE}/chat/completions").mock(side_effect=httpx.ConnectError("no dns"))
    with pytest.raises(ProviderError):
        await adapter.chat(PAYLOAD)


@respx.mock
async def test_stream_yields_chunks_until_done(adapter: Any) -> None:
    respx.post(f"{BASE}/chat/completions").respond(
        status_code=200,
        content=SSE_BODY.encode(),
        headers={"content-type": "text/event-stream"},
    )
    stream_payload = {**PAYLOAD, "stream": True}
    chunks: list[dict[str, Any]] = []
    agen = adapter.chat_stream(stream_payload)
    assert isinstance(agen, AsyncIterator)
    async for chunk in agen:
        chunks.append(chunk)
    assert len(chunks) == 2
    text = "".join(c["choices"][0]["delta"]["content"] for c in chunks)
    assert text == "Hey"


@respx.mock
async def test_stream_error_status_raises(adapter: Any) -> None:
    respx.post(f"{BASE}/chat/completions").respond(status_code=500)
    with pytest.raises(ProviderError):
        async for _ in adapter.chat_stream({**PAYLOAD, "stream": True}):
            pass

"""Live smoke tests against real providers. Run explicitly: pytest -m live."""

import os

import pytest

from fwllm.adapters.openrouter import OpenRouterAdapter

OPENROUTER_KEY = os.environ.get("OPENROUTER_API_KEY")

pytestmark = [
    pytest.mark.live,
    pytest.mark.skipif(OPENROUTER_KEY is None, reason="OPENROUTER_API_KEY not set"),
]

MODELS_URL = "https://openrouter.ai/api/v1/models"


def _pick_free_model() -> str:
    """Pick any currently free model - the :free catalog changes over time."""
    import httpx

    response = httpx.get(MODELS_URL, timeout=30.0)
    response.raise_for_status()
    free = [
        m["id"]
        for m in response.json()["data"]
        if m["id"].endswith(":free")
        and int(m.get("context_length", 0)) >= 4096
    ]
    if not free:
        pytest.skip("no free models available on OpenRouter right now")
    return sorted(free)[0]


def _adapter() -> OpenRouterAdapter:
    import httpx

    return OpenRouterAdapter(
        client=httpx.AsyncClient(
            base_url="https://openrouter.ai/api/v1", timeout=60.0
        ),
        api_key=OPENROUTER_KEY,
    )


async def test_openrouter_free_model_chat() -> None:
    adapter = _adapter()
    result = await adapter.chat(
        {
            "model": _pick_free_model(),
            "messages": [{"role": "user", "content": "Reply with exactly: OK"}],
        }
    )
    assert result["object"] == "chat.completion"
    assert result["choices"][0]["message"]["content"]
    assert result["usage"]["total_tokens"] > 0
    await adapter.aclose()

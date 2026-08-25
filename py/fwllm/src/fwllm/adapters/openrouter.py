"""OpenRouter adapter."""

from __future__ import annotations

from fwllm.adapters.openai_compat import OpenAICompatAdapter


class OpenRouterAdapter(OpenAICompatAdapter):
    """https://openrouter.ai/api/v1 - OpenAI-compatible with attribution headers."""

    def __init__(self, *args: object, **kwargs: object) -> None:
        kwargs.setdefault(
            "extra_headers",
            {
                # Recommended by OpenRouter for app attribution.
                "HTTP-Referer": "https://github.com/SoldatovAlexander/Firewall-LLM",
                "X-Title": "Firewall LLM",
            },
        )
        super().__init__(*args, **kwargs)  # type: ignore[arg-type]

"""Provider registry: builds adapters from config."""

from __future__ import annotations

from fwllm.adapters.ollama import OllamaAdapter
from fwllm.adapters.openai_compat import OpenAICompatAdapter
from fwllm.adapters.openrouter import OpenRouterAdapter
from fwllm.config import Config, ConfigError
from fwllm.egress import build_http_client
from fwllm.providers.base import Provider

_ADAPTERS: dict[str, type[OpenAICompatAdapter]] = {
    "openai_compat": OpenAICompatAdapter,
    "openrouter": OpenRouterAdapter,
    "ollama": OllamaAdapter,
}


def build_providers(config: Config) -> dict[str, Provider]:
    """Instantiate configured provider adapters."""
    providers: dict[str, Provider] = {}
    for name, pcfg in config.providers.items():
        cls = _ADAPTERS.get(pcfg.type)
        if cls is None:
            raise ConfigError(f"unknown provider type '{pcfg.type}' for provider '{name}'")
        client = build_http_client(
            pcfg.base_url,
            config.egress.proxy_url if config.egress.mode == "single_proxy" else None,
            timeout=config.server.request_timeout_seconds,
        )
        providers[name] = cls(
            client=client,
            api_key=pcfg.api_key,
            timeout=config.server.request_timeout_seconds,
        )
    return providers

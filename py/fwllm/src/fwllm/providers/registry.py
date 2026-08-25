"""Provider registry: builds adapters from config."""

from __future__ import annotations

from typing import Any

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


def _build_enterprise_manager(
    config: Config, adapter_name: str
) -> Any | None:
    """Enterprise hook: proxy pool manager for the adapter's binding."""
    if config.egress.mode != "pools":
        return None
    try:
        from fwllm_enterprise import egress_pools
    except ImportError as exc:  # pragma: no cover - packaging guard
        raise ConfigError(
            "egress.mode=pools requires the fwllm-enterprise package "
            "(commercial license)"
        ) from exc
    pool_name = config.egress.bindings.get(adapter_name)
    if pool_name is None:
        raise ConfigError(f"egress.bindings is missing a pool for adapter '{adapter_name}'")
    pool_cfg = config.egress.pools[pool_name]
    return egress_pools.build_pool(pool_cfg)


def build_providers(config: Config) -> dict[str, Provider]:
    """Instantiate configured provider adapters."""
    providers: dict[str, Provider] = {}
    for name, pcfg in config.providers.items():
        cls = _ADAPTERS.get(pcfg.type)
        if cls is None:
            raise ConfigError(f"unknown provider type '{pcfg.type}' for provider '{name}'")
        manager = _build_enterprise_manager(config, name)
        if manager is not None:
            adapter = cls(
                api_key=pcfg.api_key,
                base_url=pcfg.base_url,
                timeout=config.server.request_timeout_seconds,
                proxy_manager=manager,
            )
        else:
            client = build_http_client(
                pcfg.base_url,
                config.egress.proxy_url if config.egress.mode == "single_proxy" else None,
                timeout=config.server.request_timeout_seconds,
            )
            adapter = cls(
                client=client,
                api_key=pcfg.api_key,
                timeout=config.server.request_timeout_seconds,
            )
        providers[name] = adapter
    return providers

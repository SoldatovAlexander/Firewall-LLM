"""Registry tests: config -> adapter instances."""

import pytest

from fwllm.adapters.ollama import OllamaAdapter
from fwllm.adapters.openai_compat import OpenAICompatAdapter
from fwllm.adapters.openrouter import OpenRouterAdapter
from fwllm.config import Config, ConfigError, ProviderConfig
from fwllm.providers.registry import build_providers


def _cfg(**providers: ProviderConfig) -> Config:
    return Config(providers=dict(providers))


def test_build_openrouter_adapter():
    cfg = _cfg(
        openrouter=ProviderConfig(
            type="openrouter",
            base_url="https://openrouter.ai/api/v1",
            api_key="sk-or-test",
        )
    )
    providers = build_providers(cfg)
    assert isinstance(providers["openrouter"], OpenRouterAdapter)


def test_build_ollama_adapter():
    cfg = _cfg(
        local=ProviderConfig(type="ollama", base_url="http://localhost:11434/v1")
    )
    providers = build_providers(cfg)
    assert isinstance(providers["local"], OllamaAdapter)


def test_default_type_is_openai_compat():
    cfg = _cfg(generic=ProviderConfig(base_url="https://x.example/v1"))
    providers = build_providers(cfg)
    assert isinstance(providers["generic"], OpenAICompatAdapter)


def test_unknown_type_raises():
    # bypass pydantic validation to hit the registry's own guard
    pcfg = ProviderConfig.model_construct(type="alien", base_url="https://x.example/v1")
    cfg = _cfg(weird=pcfg)
    with pytest.raises(ConfigError, match="alien"):
        build_providers(cfg)

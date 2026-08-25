"""Provider registry: builds adapters from config (filled in phase 2)."""

from __future__ import annotations

from fwllm.config import Config
from fwllm.providers.base import Provider


def build_providers(config: Config) -> dict[str, Provider]:
    """Instantiate configured provider adapters."""
    raise NotImplementedError("provider registry arrives in phase 2")

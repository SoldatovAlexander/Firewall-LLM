"""Ollama adapter (local, on-prem inference)."""

from __future__ import annotations

from fwllm.adapters.openai_compat import OpenAICompatAdapter


class OllamaAdapter(OpenAICompatAdapter):
    """Ollama's OpenAI-compatible endpoint, typically http://host:11434/v1."""

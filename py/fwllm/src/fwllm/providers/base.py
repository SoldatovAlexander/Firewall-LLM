"""Gateway protocol shared by all provider adapters."""

from __future__ import annotations

from collections.abc import AsyncIterator
from typing import Any, Protocol, runtime_checkable


class ProviderError(Exception):
    """Raised when an upstream provider call fails."""

    def __init__(self, message: str, *, status: int | None = None):
        super().__init__(message)
        self.status = status


class BlockedError(Exception):
    """Raised when an inspector blocks the request."""

    def __init__(self, message: str, *, reason: str = "policy_violation"):
        super().__init__(message)
        self.reason = reason


@runtime_checkable
class Provider(Protocol):
    """A single external LLM API adapter."""

    async def chat(self, payload: dict[str, Any]) -> dict[str, Any]:
        """Execute a non-streaming chat completion."""
        ...

    def chat_stream(self, payload: dict[str, Any]) -> AsyncIterator[dict[str, Any]]:
        """Yield streaming chunks for a chat completion."""
        ...

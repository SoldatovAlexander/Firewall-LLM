"""OpenAI-compatible transport shared by adapters."""

from __future__ import annotations

import json
from collections.abc import AsyncIterator
from typing import Any

import httpx

from fwllm.providers.base import ProviderError


class OpenAICompatAdapter:
    """Adapter for OpenAI-style /chat/completions APIs.

    Subclasses only adjust defaults (base URL, extra headers).
    """

    def __init__(
        self,
        client: httpx.AsyncClient,
        api_key: str | None = None,
        *,
        timeout: float = 120.0,
        extra_headers: dict[str, str] | None = None,
    ):
        self._client = client
        self._api_key = api_key
        self._timeout = timeout
        self._extra_headers = extra_headers or {}

    def _headers(self) -> dict[str, str]:
        headers = dict(self._extra_headers)
        if self._api_key:
            headers["Authorization"] = f"Bearer {self._api_key}"
        return headers

    async def aclose(self) -> None:
        await self._client.aclose()

    async def chat(self, payload: dict[str, Any]) -> dict[str, Any]:
        try:
            response = await self._client.post(
                "/chat/completions",
                json=payload,
                headers=self._headers(),
                timeout=self._timeout,
            )
        except httpx.HTTPError as exc:
            raise ProviderError(f"provider connection failed: {exc}") from exc
        if response.status_code >= 400:
            raise ProviderError(
                f"provider returned {response.status_code}: {response.text[:200]}",
                status=response.status_code,
            )
        try:
            data: dict[str, Any] = response.json()
            return data
        except ValueError as exc:
            raise ProviderError("provider returned non-JSON body") from exc

    async def chat_stream(self, payload: dict[str, Any]) -> AsyncIterator[dict[str, Any]]:
        stream_payload = {**payload, "stream": True}
        try:
            async with self._client.stream(
                "POST",
                "/chat/completions",
                json=stream_payload,
                headers={**self._headers(), "Accept": "text/event-stream"},
                timeout=self._timeout,
            ) as response:
                if response.status_code >= 400:
                    body = (await response.aread()).decode(errors="replace")
                    raise ProviderError(
                        f"provider returned {response.status_code}: {body[:200]}",
                        status=response.status_code,
                    )
                async for line in response.aiter_lines():
                    if not line.startswith("data: "):
                        continue
                    data = line.removeprefix("data: ").strip()
                    if data == "[DONE]":
                        return
                    try:
                        yield json.loads(data)
                    except ValueError:
                        continue
        except httpx.HTTPError as exc:
            raise ProviderError(f"provider connection failed: {exc}") from exc

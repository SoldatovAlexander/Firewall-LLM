"""OpenAI-compatible transport shared by adapters."""

from __future__ import annotations

import json
from collections.abc import AsyncIterator
from typing import Any, Protocol

import httpx

from fwllm.providers.base import ProviderError


class EgressManager(Protocol):
    """Enterprise hook: yields the active proxy for this adapter."""

    def current(self) -> str | None: ...
    def mark(self) -> None: ...
    def report_failure(self, url: str) -> None: ...


class OpenAICompatAdapter:
    """Adapter for OpenAI-style /chat/completions APIs.

    Subclasses only adjust defaults (base URL, extra headers).

    Either a ready-made `client` is injected (tests, static egress), or a
    `proxy_manager` plus `base_url` so HTTP clients are built per active proxy
    and rebuilt transparently when the pool rotates.
    """

    def __init__(
        self,
        client: httpx.AsyncClient | None = None,
        api_key: str | None = None,
        *,
        base_url: str | None = None,
        timeout: float = 120.0,
        extra_headers: dict[str, str] | None = None,
        proxy_manager: EgressManager | None = None,
    ):
        if client is None and (base_url is None or proxy_manager is None):
            raise ValueError("provide either 'client' or 'base_url' + 'proxy_manager'")
        self._api_key = api_key
        self._timeout = timeout
        self._extra_headers = extra_headers or {}
        self._base_url = base_url
        self._proxy_manager = proxy_manager
        self._clients: dict[str, httpx.AsyncClient] = {}
        if client is not None:
            self._clients["__fixed__"] = client

    def _headers(self) -> dict[str, str]:
        headers = dict(self._extra_headers)
        if self._api_key:
            headers["Authorization"] = f"Bearer {self._api_key}"
        return headers

    def _ensure_client(self) -> httpx.AsyncClient:
        if self._proxy_manager is None:
            return self._clients["__fixed__"]
        proxy = self._proxy_manager.current()
        key = proxy or "__direct__"
        if key not in self._clients:
            kwargs: dict[str, Any] = {
                "base_url": self._base_url or "",
                "timeout": self._timeout,
            }
            if proxy:
                kwargs["proxy"] = proxy
                kwargs["trust_env"] = False
            self._clients[key] = httpx.AsyncClient(**kwargs)
        return self._clients[key]

    async def aclose(self) -> None:
        for client in self._clients.values():
            await client.aclose()

    async def _request_chat(self, payload: dict[str, Any]) -> httpx.Response:
        client = self._ensure_client()
        proxy_url = (
            self._proxy_manager.current() if self._proxy_manager else None
        )
        try:
            response = await client.post(
                "/chat/completions",
                json=payload,
                headers=self._headers(),
                timeout=self._timeout,
            )
        except httpx.HTTPError as exc:
            if self._proxy_manager and proxy_url:
                self._proxy_manager.report_failure(proxy_url)
            raise ProviderError(f"provider connection failed: {exc}") from exc
        if self._proxy_manager:
            self._proxy_manager.mark()
        return response

    async def chat(self, payload: dict[str, Any]) -> dict[str, Any]:
        response = await self._request_chat(payload)
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
        client = self._ensure_client()
        proxy_url = (
            self._proxy_manager.current() if self._proxy_manager else None
        )
        try:
            async with client.stream(
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
                if self._proxy_manager:
                    self._proxy_manager.mark()
        except httpx.HTTPError as exc:
            if self._proxy_manager and proxy_url:
                self._proxy_manager.report_failure(proxy_url)
            raise ProviderError(f"provider connection failed: {exc}") from exc

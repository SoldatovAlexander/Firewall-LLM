"""Egress: outbound connection policy for provider adapters.

MVP edge cases only:
- direct      - every adapter connects without a proxy
- single_proxy - every adapter goes through one global proxy

Multi-pool rotation, per-adapter bindings and healthchecks are enterprise.
"""

from __future__ import annotations

from typing import Any

import httpx


def build_http_client(
    base_url: str, proxy_url: str | None, timeout: float = 120.0
) -> httpx.AsyncClient:
    kwargs: dict[str, Any] = {"base_url": base_url, "timeout": timeout}
    if proxy_url:
        kwargs["proxy"] = proxy_url
        # trust_env off: explicit proxy must not be overridden by env vars
        kwargs["trust_env"] = False
    return httpx.AsyncClient(**kwargs)

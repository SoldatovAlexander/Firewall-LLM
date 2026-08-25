"""FastAPI application: unified OpenAI-compatible entry point."""

from __future__ import annotations

import json
import logging
import time
from collections.abc import AsyncIterator, Awaitable
from typing import Annotated, Any, TypeVar

from fastapi import Depends, FastAPI, Request
from fastapi.exceptions import RequestValidationError
from fastapi.responses import StreamingResponse
from prometheus_client import make_asgi_app
from pydantic import BaseModel, Field

from fwllm.config import Config
from fwllm.errors import (
    ApiError,
    auth_error,
    blocked_error,
    rate_limit_error,
    upstream_error,
    validation_error_handler,
)
from fwllm.metering import Metering, QuotaExceeded
from fwllm.observability.metrics import observe_request
from fwllm.providers.base import BlockedError, Provider, ProviderError

T = TypeVar("T")

logger = logging.getLogger(__name__)


async def _metering_safe(op: Awaitable[T]) -> T | None:
    """Metering backend outages must never break traffic (fail-open MVP policy)."""
    try:
        return await op
    except QuotaExceeded:
        raise
    except Exception:  # noqa: BLE001
        logger.warning("metering backend unavailable, skipping accounting")
        return None


class ChatMessage(BaseModel):
    role: str
    content: str | None = None


class ChatCompletionRequest(BaseModel):
    model: str
    messages: list[ChatMessage] = Field(min_length=1)
    stream: bool = False
    temperature: float | None = None
    top_p: float | None = None
    max_tokens: int | None = None

    def to_payload(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "model": self.model,
            "messages": [m.model_dump(exclude_none=True) for m in self.messages],
            "stream": self.stream,
        }
        for opt in ("temperature", "top_p", "max_tokens"):
            value = getattr(self, opt)
            if value is not None:
                payload[opt] = value
        return payload


def _resolve_provider(config: Config, providers: dict[str, Provider]) -> Provider:
    """Phase 1: single-provider selection. Replaced by router in phase 7."""
    if not providers:
        raise RuntimeError("no providers configured")
    first = next(iter(providers))
    return providers[first]


async def _require_client(request: Request) -> str:
    clients: dict[str, str] = request.app.state.clients
    auth = request.headers.get("Authorization", "")
    if not auth.startswith("Bearer "):
        raise auth_error("missing bearer token", code="missing_api_key")
    token = auth.removeprefix("Bearer ").strip()
    if not token or (clients and token not in clients):
        raise auth_error("invalid API key", code="invalid_api_key")
    if not clients:
        raise auth_error("no clients configured", code="invalid_api_key")
    return clients.get(token, token)


def create_app(
    config: Config,
    providers: dict[str, Provider] | None = None,
    metering: Metering | None = None,
) -> FastAPI:
    app = FastAPI(title="Firewall LLM", version="0.1.0")
    app.state.config = config
    app.state.clients = config.clients
    if providers is None:
        from fwllm.providers.registry import build_providers

        providers = build_providers(config)
    app.state.providers = providers
    provider = _resolve_provider(config, providers)
    if metering is None:
        import redis.asyncio as aioredis

        metering = Metering(
            aioredis.from_url(config.redis_url, decode_responses=True),
            quotas=config.quotas.model_dump(exclude_none=True),
        )
    app.state.metering = metering

    async def validation_handler(_request: Request, exc: RequestValidationError) -> Any:
        return await validation_error_handler(_request, exc)

    app.add_exception_handler(
        RequestValidationError, validation_handler  # type: ignore[arg-type]
    )

    async def api_error_handler(_request: Request, exc: ApiError) -> Any:
        return exc.response()

    app.add_exception_handler(ApiError, api_error_handler)  # type: ignore[arg-type]

    @app.get("/healthz")
    async def healthz() -> dict[str, str]:
        return {"status": "ok"}

    @app.post("/v1/chat/completions")
    async def chat_completions(
        body: ChatCompletionRequest,
        client_id: Annotated[str, Depends(_require_client)],
    ) -> Any:
        payload = body.to_payload()
        provider_name = next(iter(app.state.providers), "unknown")
        started = time.monotonic()

        def _metrics(code: str, prompt: int = 0, completion: int = 0) -> None:
            observe_request(
                client=client_id,
                provider=provider_name,
                model=body.model,
                code=code,
                duration=time.monotonic() - started,
                prompt=prompt,
                completion=completion,
            )

        if not body.stream:
            try:
                await _metering_safe(metering.check_client(client_id))
            except QuotaExceeded as exc:
                _metrics("rate_limited")
                raise rate_limit_error(str(exc)) from exc
            try:
                result = await provider.chat(payload)
            except BlockedError as exc:
                _metrics("blocked")
                raise blocked_error(str(exc), reason=exc.reason) from exc
            except ProviderError as exc:
                _metrics("upstream_error")
                raise upstream_error(str(exc)) from exc
            usage = result.get("usage") or {}
            prompt_tokens = int(usage.get("prompt_tokens", 0))
            completion_tokens = int(usage.get("completion_tokens", 0))
            await _metering_safe(
                metering.record(
                    client=client_id,
                    provider=provider_name,
                    model=body.model,
                    prompt=prompt_tokens,
                    completion=completion_tokens,
                )
            )
            _metrics("ok", prompt=prompt_tokens, completion=completion_tokens)
            return result

        async def sse() -> AsyncIterator[str]:
            code = "ok"
            try:
                async for chunk in provider.chat_stream(payload):
                    yield f"data: {json.dumps(chunk, separators=(',', ':'))}\n\n"
            except BlockedError as exc:
                code = "blocked"
                err = blocked_error(str(exc), reason=exc.reason)
                yield f"data: {json.dumps(err.as_dict(), separators=(',', ':'))}\n\n"
                return
            except ProviderError as exc:
                code = "upstream_error"
                err = upstream_error(str(exc))
                yield f"data: {json.dumps(err.as_dict(), separators=(',', ':'))}\n\n"
                return
            finally:
                _metrics(code)
            yield "data: [DONE]\n\n"

        return StreamingResponse(sse(), media_type="text/event-stream")

    app.mount("/metrics", make_asgi_app())
    return app

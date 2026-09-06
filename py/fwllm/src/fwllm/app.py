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
from pydantic import BaseModel, Field

from fwllm.audit import AuditLog, ensure_parent
from fwllm.config import Config
from fwllm.errors import (
    ApiError,
    auth_error,
    blocked_error,
    rate_limit_error,
    upstream_error,
    validation_error_handler,
)
from fwllm.inspectors.chain import InspectorChain
from fwllm.metering import Metering, QuotaExceeded, estimate_usage
from fwllm.observability.metrics import observe_request
from fwllm.providers.base import BlockedError, Provider, ProviderError
from fwllm.router.policy import PolicyEngine

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
    name: str | None = None
    tool_calls: list[dict[str, Any]] | None = None


class ChatCompletionRequest(BaseModel):
    model: str
    messages: list[ChatMessage] = Field(min_length=1)
    stream: bool = False
    temperature: float | None = Field(default=None, ge=0, le=2)
    top_p: float | None = Field(default=None, ge=0, le=1)
    max_tokens: int | None = Field(default=None, ge=1)
    stop: str | list[str] | None = None
    # Client-side per contract; accepted but never forwarded upstream.
    metadata: dict[str, Any] | None = None
    # R03: user-supplied stream options are preserved and forwarded upstream.
    stream_options: dict[str, Any] | None = None

    def to_payload(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "model": self.model,
            "messages": [m.model_dump(exclude_none=True) for m in self.messages],
            "stream": self.stream,
        }
        for opt in ("temperature", "top_p", "max_tokens", "stop", "stream_options"):
            value = getattr(self, opt)
            if value is not None:
                payload[opt] = value
        if self.stream:
            # R03: ask supporting providers for a terminal usage chunk so
            # streaming responses can be accounted exactly. An explicit user
            # choice is respected; absence defaults to True.
            options = dict(payload.get("stream_options") or {})
            options.setdefault("include_usage", True)
            payload["stream_options"] = options
        return payload


def _prompt_text(payload: dict[str, Any]) -> str:
    """Join request message contents for usage estimation (R03)."""
    parts: list[str] = []
    messages = payload.get("messages") or []
    if isinstance(messages, list):
        for message in messages:
            if isinstance(message, dict):
                content = message.get("content")
                if isinstance(content, str):
                    parts.append(content)
    return "\n".join(parts)


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


async def _require_admin(request: Request) -> str:
    admin_clients: dict[str, str] = getattr(request.app.state, "admin_clients", {})
    clients: dict[str, str] = request.app.state.clients
    auth = request.headers.get("Authorization", "")
    if not auth.startswith("Bearer "):
        raise auth_error("missing bearer token", code="missing_api_key")
    token = auth.removeprefix("Bearer ").strip()
    if not token:
        raise auth_error("invalid API key", code="invalid_api_key")
    if token in admin_clients:
        return admin_clients[token]
    # No fallback: an empty admin list means admin endpoints are disabled.
    # Self-audit stays available via _require_client with mandatory ID filter.
    if token in clients:
        raise ApiError(
            status=403,
            type_="permission_error",
            message="admin privileges required",
            code="admin_required",
        )
    raise auth_error("invalid API key", code="invalid_api_key")


def _build_redis_store(url: str) -> Any:
    import redis as redis_sync

    from fwllm.router.store import RedisRouterStore

    return RedisRouterStore(redis_sync.from_url(url, decode_responses=True))


def create_app(
    config: Config,
    providers: dict[str, Provider] | None = None,
    metering: Metering | None = None,
    inspectors: InspectorChain | None = None,
    router: PolicyEngine | None = None,
    audit_log: AuditLog | None = None,
) -> FastAPI:
    app = FastAPI(title="Firewall LLM", version="0.1.0")
    app.state.config = config
    app.state.clients = config.clients
    app.state.admin_clients = config.admin_clients
    if not config.admin_clients:
        logger.warning(
            "no admin_clients configured: /admin/* endpoints are disabled, "
            "clients can only self-audit; set admin_clients or FWLLM_ADMIN_TOKENS"
        )
    if providers is None:
        from fwllm.providers.registry import build_providers

        providers = build_providers(config)
    app.state.providers = providers
    if metering is None:
        import redis.asyncio as aioredis

        metering = Metering(
            aioredis.from_url(config.redis_url, decode_responses=True),
            quotas=config.quotas.model_dump(exclude_none=True),
            backend_fail_closed=config.quotas.backend_fail_closed,
        )
    app.state.metering = metering
    if inspectors is None:
        inspectors = InspectorChain.from_config(config.inspectors)

    if router is None:
        routing = config.routing
        if not routing.default_chain and providers:
            routing = routing.model_copy(
                update={"default_chain": list(providers.keys())}
            )
        PolicyEngine.validate_routing(routing, list(providers.keys()))
        store = (
            _build_redis_store(config.redis_url)
            if routing.state_store == "redis"
            else None
        )
        router = PolicyEngine(routing, store=store)
    app.state.router = router
    metering.subscribe(router.on_event)
    inspectors.set_publish(router.on_event)

    if audit_log is None:
        ensure_parent(config.audit.db_path)
        audit_log = AuditLog(config.audit)
    app.state.audit = audit_log

    def _audit_write(
        *,
        client: str,
        provider: str,
        model: str,
        code: str,
        messages: list[dict[str, Any]],
        response_text: str,
        prompt_tokens: int = 0,
        completion_tokens: int = 0,
        usage_source: str = "upstream",
    ) -> None:
        if not audit_log.enabled:
            return
        try:
            audit_log.write(
                client=client,
                provider=provider,
                model=model,
                code=code,
                prompt_tokens=prompt_tokens,
                completion_tokens=completion_tokens,
                usage_source=usage_source,
                messages=messages,
                response_text=response_text,
            )
        except Exception:  # noqa: BLE001
            logger.warning("audit write failed", exc_info=True)

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

    @app.get("/admin/audit")
    async def admin_audit(
        request: Request,
        code: str | None = None,
        limit: int = 100,
    ) -> Any:
        # Admin can see all (with optional ?client= filter), non-admin only own records
        try:
            await _require_admin(request)
            client_filter = request.query_params.get("client")
        except ApiError:
            client_id = await _require_client(request)
            client_filter = client_id
        records = audit_log.search(client=client_filter, code=code, limit=min(limit, 1000))
        return {"total": len(records), "records": records}

    @app.post("/v1/chat/completions")
    async def chat_completions(
        body: ChatCompletionRequest,
        client_id: Annotated[str, Depends(_require_client)],
    ) -> Any:
        payload = body.to_payload()
        provider_name = "unrouted"
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

        try:
            provider_name, concrete_model = router.resolve(body.model, client_id)
        except QuotaExceeded as exc:
            _metrics("rate_limited")
            _audit_write(
                client=client_id,
                provider="unrouted",
                model=body.model,
                code="rate_limited",
                messages=body.model_dump()["messages"],
                response_text=str(exc),
            )
            raise rate_limit_error(str(exc)) from exc
        except BlockedError as exc:
            _metrics("blocked")
            _audit_write(
                client=client_id,
                provider="unrouted",
                model=body.model,
                code="blocked_source",
                messages=body.model_dump()["messages"],
                response_text=str(exc),
            )
            raise blocked_error(str(exc), reason=exc.reason) from exc
        payload["model"] = concrete_model
        provider = app.state.providers.get(provider_name)
        if provider is None:
            raise upstream_error(f"routed provider '{provider_name}' not configured")

        def _audit_now(
            code: str,
            response_text: str,
            prompt_tokens: int = 0,
            completion_tokens: int = 0,
            messages: list[dict[str, Any]] | None = None,
            usage_source: str = "upstream",
        ) -> None:
            _audit_write(
                client=client_id,
                provider=provider_name,
                model=body.model,
                code=code,
                messages=payload.get("messages", []) if messages is None else messages,
                response_text=response_text,
                prompt_tokens=prompt_tokens,
                completion_tokens=completion_tokens,
                usage_source=usage_source,
            )

        # Common pre-processing for both streaming and non-streaming
        try:
            ctx = inspectors.process_request(payload, client=client_id)
        except BlockedError as exc:
            _metrics("blocked")
            _audit_now("blocked", str(exc), messages=body.model_dump()["messages"])
            raise blocked_error(str(exc), reason=exc.reason) from exc
        try:
            if metering._backend_fail_closed:
                await metering.check_client(client_id)
            else:
                await _metering_safe(metering.check_client(client_id))
        except QuotaExceeded as exc:
            _metrics("rate_limited")
            _audit_now("rate_limited", str(exc))
            raise rate_limit_error(str(exc)) from exc
        except Exception as exc:
            # fail-closed: backend unreachable -> 503 service unavailable
            _metrics("backend_error")
            _audit_now("backend_error", str(exc))
            raise ApiError(
                status=503,
                type_="rate_limit_error",
                message=f"metering backend unavailable: {exc}",
                code="backend_unavailable",
            ) from exc
        try:
            if metering._backend_fail_closed:
                await metering.check_provider(provider_name)
            else:
                await _metering_safe(metering.check_provider(provider_name))
        except QuotaExceeded as exc:
            _metrics("rate_limited")
            _audit_now("rate_limited", str(exc))
            raise rate_limit_error(str(exc)) from exc
        except Exception as exc:
            _metrics("backend_error")
            _audit_now("backend_error", str(exc))
            raise ApiError(
                status=503,
                type_="rate_limit_error",
                message=f"metering backend unavailable: {exc}",
                code="backend_unavailable",
            ) from exc

        if not body.stream:
            try:
                result = await provider.chat(payload)
            except BlockedError as exc:
                _metrics("blocked")
                _audit_now("blocked", str(exc))
                raise blocked_error(str(exc), reason=exc.reason) from exc
            except ProviderError as exc:
                _metrics("upstream_error")
                _audit_now("upstream_error", str(exc))
                raise upstream_error(str(exc)) from exc
            result = inspectors.process_response(result, ctx)
            if concrete_model != body.model:
                result["routed_from"] = body.model
            response_text = "\n".join(
                choice.get("message", {}).get("content") or ""
                for choice in result.get("choices", [])
            )
            # R03: provider usage wins; otherwise estimate from text and mark it.
            usage = result.get("usage") or {}
            if usage:
                prompt_tokens = int(usage.get("prompt_tokens", 0))
                completion_tokens = int(usage.get("completion_tokens", 0))
                usage_source = "upstream"
            else:
                prompt_tokens, completion_tokens = estimate_usage(
                    _prompt_text(payload), response_text
                )
                usage_source = "estimated"
            await _metering_safe(
                metering.record(
                    client=client_id,
                    provider=provider_name,
                    model=body.model,
                    prompt=prompt_tokens,
                    completion=completion_tokens,
                    usage_source=usage_source,
                )
            )
            _audit_now(
                "ok", response_text, prompt_tokens, completion_tokens,
                usage_source=usage_source,
            )
            _metrics("ok", prompt=prompt_tokens, completion=completion_tokens)
            return result

        async def sse() -> AsyncIterator[str]:
            code = "ok"
            response_parts: list[str] = []
            # R13: stateful restore session reassembles DLP tokens split
            # across SSE chunk boundaries.
            restore_session = inspectors.stream_restore_session(ctx)
            # For usage accounting in streaming, capture last chunk's usage
            last_usage: dict[str, Any] | None = None
            try:
                async for chunk in provider.chat_stream(payload):
                    if not isinstance(chunk, dict):
                        continue
                    # R04: usage is extracted independently of choices, so a
                    # usage-only chunk (choices=[]) cannot crash the stream.
                    usage = chunk.get("usage")
                    if isinstance(usage, dict) and usage:
                        last_usage = usage
                    choices = chunk.get("choices") or []
                    if not isinstance(choices, list):
                        choices = []
                    for choice in choices:
                        if not isinstance(choice, dict):
                            continue
                        delta_obj = choice.get("delta")
                        if not isinstance(delta_obj, dict):
                            continue
                        delta = delta_obj.get("content")
                        if not isinstance(delta, str) or not delta:
                            continue
                        # Apply stateful streaming DLP restore (R13)
                        try:
                            delta = restore_session.feed(delta)
                            delta_obj["content"] = delta
                        except Exception:
                            logger.debug("streaming DLP restore failed", exc_info=True)
                        response_parts.append(delta)
                    yield f"data: {json.dumps(chunk, separators=(',', ':'))}\n\n"
                # Flush any held-back trailing text; never drop it silently.
                try:
                    flushed = restore_session.flush()
                except Exception:
                    flushed = ""
                    logger.debug("streaming DLP flush failed", exc_info=True)
                if flushed:
                    response_parts.append(flushed)
                    tail = {
                        "id": "chatcmpl-stream",
                        "object": "chat.completion.chunk",
                        "choices": [{"index": 0, "delta": {"content": flushed}}],
                    }
                    yield f"data: {json.dumps(tail, separators=(',', ':'))}\n\n"
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
                # R03: always count the admitted request. Provider usage wins;
                # otherwise estimate from the exchanged text and mark it.
                # This single finalize site runs on completion, error and
                # client disconnect, so accounting happens exactly once.
                if last_usage:
                    prompt_tokens = int(last_usage.get("prompt_tokens", 0))
                    completion_tokens = int(last_usage.get("completion_tokens", 0))
                    usage_source = "upstream"
                else:
                    prompt_tokens, completion_tokens = estimate_usage(
                        _prompt_text(payload), "".join(response_parts)
                    )
                    usage_source = "estimated"
                try:
                    await metering.record(
                        client=client_id,
                        provider=provider_name,
                        model=body.model,
                        prompt=prompt_tokens,
                        completion=completion_tokens,
                        usage_source=usage_source,
                    )
                except Exception:
                    logger.debug("streaming metering record failed", exc_info=True)
                _metrics(
                    code,
                    prompt=prompt_tokens,
                    completion=completion_tokens,
                )
                _audit_now(
                    code, "".join(response_parts), prompt_tokens, completion_tokens,
                    usage_source=usage_source,
                )
            yield "data: [DONE]\n\n"

        return StreamingResponse(sse(), media_type="text/event-stream")

    from fastapi.responses import Response as FastAPIResponse
    from prometheus_client import CONTENT_TYPE_LATEST, generate_latest

    @app.get("/metrics")
    async def metrics(request: Request) -> FastAPIResponse:
        try:
            await _require_admin(request)
        except ApiError as exc:
            return exc.response()
        return FastAPIResponse(content=generate_latest(), media_type=CONTENT_TYPE_LATEST)

    return app

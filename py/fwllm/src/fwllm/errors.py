"""Unified API error format (see contracts/openapi.yaml)."""

from __future__ import annotations

from typing import Any

from fastapi import Request
from fastapi.exceptions import RequestValidationError
from fastapi.responses import JSONResponse


class ApiError(Exception):
    def __init__(
        self,
        *,
        status: int,
        type_: str,
        message: str,
        code: str | None = None,
        details: dict[str, Any] | None = None,
    ):
        super().__init__(message)
        self.status = status
        self.type = type_
        self.message = message
        self.code = code
        self.details = details

    def as_dict(self) -> dict[str, Any]:
        body: dict[str, Any] = {"message": self.message, "type": self.type}
        if self.code:
            body["code"] = self.code
        if self.details:
            body["details"] = self.details
        return {"error": body}

    def response(self) -> JSONResponse:
        return JSONResponse(status_code=self.status, content=self.as_dict())


def auth_error(message: str, code: str) -> ApiError:
    return ApiError(status=401, type_="authentication_error", message=message, code=code)


def blocked_error(message: str, reason: str) -> ApiError:
    return ApiError(
        status=403,
        type_="permission_error",
        message=message,
        code="blocked_by_inspector",
        details={"reason": reason},
    )


def rate_limit_error(message: str) -> ApiError:
    return ApiError(status=429, type_="rate_limit_error", message=message)


def upstream_error(message: str) -> ApiError:
    return ApiError(status=502, type_="upstream_error", message=message)


async def validation_error_handler(
    _request: Request, exc: RequestValidationError
) -> JSONResponse:
    return ApiError(
        status=422,
        type_="invalid_request_error",
        message="invalid request body",
        code="invalid_body",
        details={"errors": exc.errors()},
    ).response()

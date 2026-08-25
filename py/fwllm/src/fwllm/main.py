"""Production entrypoint: uvicorn fwllm.main:app

`app` is resolved lazily (PEP 562) so importing this module has no side effects.
"""

from __future__ import annotations

import os
from typing import Any

from fastapi import FastAPI

from fwllm.app import create_app
from fwllm.config import load_config


def create_app_from_env() -> FastAPI:
    config_path = os.environ.get("FWLLM_CONFIG")
    if not config_path:
        raise RuntimeError(
            "FWLLM_CONFIG environment variable must point to a fwllm.yaml file"
        )
    return create_app(load_config(config_path))


def __getattr__(name: str) -> Any:
    if name == "app":
        return create_app_from_env()
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")

"""Signature-based prompt injection detector."""

from __future__ import annotations

import logging
import re
import threading
from collections.abc import Callable
from typing import Any

from fwllm.config import InjectionConfig
from fwllm.metering import Event
from fwllm.providers.base import BlockedError

logger = logging.getLogger(__name__)

SEVERITY_ORDER: dict[str, int] = {"low": 0, "medium": 1, "high": 2, "critical": 3}

SIGNATURES: list[tuple[str, str, re.Pattern[str]]] = [
    (
        "override_instructions",
        "critical",
        re.compile(
            r"ignore\s+(all\s+)?(previous|prior|above|earlier)\s+instructions"
            r"|disregard\s+(all\s+)?(previous|prior|above)"
            r"|(reveal|print|show)\s+(your\s+)?(system\s+)?(prompt|instructions)",
            re.IGNORECASE,
        ),
    ),
    (
        "jailbreak_persona",
        "high",
        re.compile(
            r"\bjailbreak\b|\bDAN\s+mode\b|developer\s+mode"
            r"|you\s+are\s+now\s+(a|an|no\s+longer)",
            re.IGNORECASE,
        ),
    ),
    (
        "roleplay_probe",
        "medium",
        re.compile(
            r"pretend\s+(you\s+are|to\s+be)|act\s+as\s+(if|a|an)\b"
            r"|without\s+(any\s+)?restrictions",
            re.IGNORECASE,
        ),
    ),
]


def handle_findings(
    findings: list[tuple[str, str]],
    config: InjectionConfig,
    publish: Callable[[Event], None],
    detector: str,
    client: str | None,
) -> None:
    """Shared verdict logic for all injection detectors."""
    if config.mode == "off" or not findings:
        return
    rule, severity = max(findings, key=lambda f: SEVERITY_ORDER[f[1]])
    publish(
        Event(
            "attack_detected",
            {
                "kind": "prompt_injection",
                "rule": rule,
                "severity": severity,
                "detector": detector,
                "client": client,
            },
        )
    )
    if (
        config.mode == "block"
        and SEVERITY_ORDER[severity]
        >= SEVERITY_ORDER[config.block_severity_gte]
    ):
        raise BlockedError(
            f"prompt injection detected ({rule}, severity={severity})",
            reason="injection",
        )


class InjectionInspector:
    def __init__(
        self,
        config: InjectionConfig,
        publish: Callable[[Event], None] | None = None,
    ):
        self._config = config
        self._publish = publish or (lambda event: None)

    def process_request(self, payload: dict[str, Any], client: str | None = None) -> None:
        self.inspect_messages(payload.get("messages", []), client=client)

    def inspect_messages(
        self, messages: list[dict[str, Any]], client: str | None = None
    ) -> None:
        if self._config.mode == "off":
            return
        findings = self._scan(messages)
        handle_findings(findings, self._config, self._publish, "signatures", client)

    def _scan(self, messages: list[dict[str, Any]]) -> list[tuple[str, str]]:
        findings: list[tuple[str, str]] = []
        for message in messages:
            content = message.get("content")
            if not isinstance(content, str):
                continue
            for name, severity, pattern in SIGNATURES:
                if pattern.search(content):
                    findings.append((name, severity))
        return findings

    def process_response(self, text: str, part: None) -> str:
        return text


class RateLimiter:
    """Tiny per-key sliding-window limiter used to cap ML inference cost."""

    def __init__(self, max_calls: int = 100, window_seconds: float = 60.0):
        self._max = max_calls
        self._window = window_seconds
        self._calls: dict[str, list[float]] = {}
        self._lock = threading.Lock()

    def allow(self, key: str, now_ts: float) -> bool:
        with self._lock:
            bucket = [ts for ts in self._calls.get(key, []) if now_ts - ts < self._window]
            if len(bucket) >= self._max:
                self._calls[key] = bucket
                return False
            bucket.append(now_ts)
            self._calls[key] = bucket
            return True

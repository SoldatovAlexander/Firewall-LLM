"""DLP inspector backed by LightAnon reversible sanitization."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass, field
from typing import Any

from lightanon.rag import TextSanitizer

from fwllm.config import DLPConfig
from fwllm.metering import Event
from fwllm.providers.base import BlockedError


@dataclass
class DLPState:
    sanitizer: TextSanitizer | None = None
    scope: dict[str, int] = field(default_factory=dict)


class DLPInspector:
    def __init__(
        self,
        config: DLPConfig,
        publish: Callable[[Event], None] | None = None,
    ):
        self._config = config
        self._publish = publish or (lambda event: None)

    def process_request(
        self, payload: dict[str, Any], client: str | None = None
    ) -> DLPState:
        state = DLPState()
        if self._config.mode == "off":
            return state
        # fresh vault per request: tokens never outlive the exchange
        state.sanitizer = TextSanitizer(profile=self._config.profile)
        total_redacted = 0
        for message in payload.get("messages", []):
            content = message.get("content")
            if not isinstance(content, str):
                continue
            if self._config.mode == "block":
                report = state.sanitizer.scan(content)
                if report["total"]:
                    raise BlockedError(
                        "sensitive data detected in request (DLP block mode)",
                        reason="dlp",
                    )
            elif self._config.mode == "mask":
                report = state.sanitizer.scan(content)
                clean, scope = state.sanitizer.sanitize_with_scope(content)
                total_redacted += int(report["total"])
                for token, count in scope.items():
                    state.scope[token] = state.scope.get(token, 0) + count
                message["content"] = clean
        if total_redacted:
            self._publish(
                Event(
                    "dlp_redacted",
                    {"total": total_redacted, "mode": "mask", "client": client},
                )
            )
        return state

    def process_response(self, text: str, part: DLPState) -> str:
        if self._config.mode == "off" or part.sanitizer is None:
            return text
        policy = "restore" if self._config.restore_policy == "restore" else "mask"
        scope = part.scope if policy == "restore" else None
        restored: str = part.sanitizer.deanonymize(text, policy=policy, token_scope=scope)
        return restored

    def restore_stream_text(self, text: str, part: DLPState) -> str:
        """Same as process_response; explicit alias for streaming use."""
        return self.process_response(text, part)

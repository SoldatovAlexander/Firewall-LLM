"""DLP inspector backed by LightAnon reversible sanitization."""

from __future__ import annotations

import re
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
            elif self._config.mode == "log":
                report = state.sanitizer.scan(content)
                total_redacted += int(report["total"])
        if total_redacted:
            self._publish(
                Event(
                    "dlp_redacted",
                    {"total": total_redacted, "mode": self._config.mode, "client": client},
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

    def stream_session(self, part: DLPState) -> StreamRestore:
        """Stateful restore session for one streamed response (R13)."""
        return StreamRestore(self, part)


# Matches a trailing, possibly incomplete LightAnon token such as
# "[EMAIL_ab12" at end of buffer (no closing bracket yet).
_PARTIAL_TOKEN_RE = re.compile(r"\[[A-Za-z0-9_]{0,64}$")
# Safety cap so a never-completing "[" cannot grow memory unboundedly.
_MAX_CARRY = 256


class StreamRestore:
    """Reassembles DLP tokens split across SSE chunk boundaries (R13).

    feed() holds back a trailing partial token and restores the complete
    head; flush() emits whatever is left (never silently drops text).
    Incomplete tokens do not match the vault, so they pass through as-is.
    """

    def __init__(self, inspector: DLPInspector, part: DLPState):
        self._inspector = inspector
        self._part = part
        self._carry = ""

    def feed(self, text: str) -> str:
        buf = self._carry + text
        self._carry = ""
        match = _PARTIAL_TOKEN_RE.search(buf)
        head = buf
        if match is not None:
            head, self._carry = buf[: match.start()], buf[match.start() :]
        if len(self._carry) > _MAX_CARRY:
            # Never-completing bracket: emit as plain text, keep it bounded.
            head += self._carry
            self._carry = ""
        return self._inspector.process_response(head, self._part)

    def flush(self) -> str:
        tail, self._carry = self._carry, ""
        if not tail:
            return ""
        return self._inspector.process_response(tail, self._part)

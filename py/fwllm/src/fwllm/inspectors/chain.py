"""Inspector chain: request/response pipeline for safety inspections."""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass, field
from typing import Any

from fwllm.config import InspectorsConfig
from fwllm.inspectors.dlp import DLPInspector
from fwllm.inspectors.injection import InjectionInspector
from fwllm.metering import Event


@dataclass
class InspectionContext:
    """Carries per-request inspection state through to the response phase."""

    parts: list[Any] = field(default_factory=list)

    @property
    def scope(self) -> dict[str, int]:
        """Merged token scope across inspectors (for restore policies)."""
        merged: dict[str, int] = {}
        for part in self.parts:
            scope = getattr(part, "scope", None)
            if isinstance(scope, dict):
                for token, count in scope.items():
                    merged[token] = merged.get(token, 0) + count
        return merged


class InspectorChain:
    def __init__(
        self,
        inspectors: list[Any],
        publish: Callable[[Event], None] | None = None,
    ):
        self._inspectors = inspectors
        self._publish = publish

    @classmethod
    def from_config(
        cls, config: InspectorsConfig, publish: Callable[[Event], None] | None = None
    ) -> InspectorChain:
        return cls(
            [
                InjectionInspector(config.injection, publish=publish),
                DLPInspector(config.dlp, publish=publish),
            ],
            publish=publish,
        )

    def set_publish(self, publish: Callable[[Event], None] | None) -> None:
        """Re-wire event publishing (used by gateway to feed the policy engine)."""
        self._publish = publish
        for inspector in self._inspectors:
            set_pub = getattr(inspector, "_publish", None)
            if set_pub is not None:
                inspector._publish = publish  # noqa: SLF001

    def process_request(
        self, payload: dict[str, Any], client: str | None = None
    ) -> InspectionContext:
        parts: list[Any] = []
        for inspector in self._inspectors:
            try:
                parts.append(inspector.process_request(payload, client=client))
            except TypeError:
                parts.append(inspector.process_request(payload))
        return InspectionContext(parts=parts)

    def process_response(self, result: Any, ctx: InspectionContext) -> Any:
        if isinstance(result, str):
            return self._process_text(result, ctx)
        for choice in result.get("choices", []):
            content = choice.get("message", {}).get("content")
            if isinstance(content, str):
                choice["message"]["content"] = self._process_text(content, ctx)
        return result

    def _process_text(self, text: str, ctx: InspectionContext) -> str:
        for inspector, part in zip(self._inspectors, ctx.parts, strict=False):
            text = inspector.process_response(text, part)
        return text

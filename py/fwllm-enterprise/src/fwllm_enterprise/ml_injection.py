"""Enterprise ML injection detector: local ONNX classifier (commercial).

Runs fully on-prem; the model is loaded from a local directory
(``model.onnx`` + ``tokenizer.json`` + ``config.json``) - no external calls.
"""

from __future__ import annotations

import json
import logging
from collections.abc import Callable
from pathlib import Path
from typing import Any, Protocol

import numpy as np

from fwllm.config import InjectionConfig
from fwllm.inspectors.injection import handle_findings
from fwllm.metering import Event

logger = logging.getLogger(__name__)

_SEVERITY_CUTS: list[tuple[float, str]] = [
    (0.9, "critical"),
    (0.8, "high"),
    (0.7, "medium"),
]


def _severity_from_confidence(confidence: float) -> str:
    for cut, severity in _SEVERITY_CUTS:
        if confidence >= cut:
            return severity
    return "low"


class Tokenizer(Protocol):
    def encode(self, text: str) -> Any: ...


class OnnxTextClassifier:
    """ONNX sequence classifier wrapper (softmax over logits)."""

    def __init__(
        self,
        session: Any,
        tokenizer: Any,
        id2label: dict[int, str],
        max_length: int = 256,
    ):
        self._session = session
        self._tokenizer = tokenizer
        self._id2label = {int(k): v for k, v in id2label.items()}
        self._max_length = max_length
        self._input_names = [i.name for i in session.get_inputs()]

    def predict(self, text: str) -> tuple[str, float]:
        encoded = self._tokenizer.encode(text)
        ids = list(encoded.ids)[: self._max_length]
        mask = [1] * len(ids)
        feeds_by_name: dict[str, Any] = {}
        for name in self._input_names:
            if name == "input_ids" or name.endswith("input_ids"):
                feeds_by_name[name] = np.array([ids], dtype=np.int64)
            elif "mask" in name:
                feeds_by_name[name] = np.array([mask], dtype=np.int64)
            else:
                feeds_by_name[name] = np.zeros((1, len(ids)), dtype=np.int64)
        logits = self._session.run(None, feeds_by_name)[0][0]
        exps = np.exp(logits - logits.max())
        probs = exps / exps.sum()
        best = int(np.argmax(probs))
        label = self._id2label.get(best, str(best))
        return label, float(probs[best])


def try_load_classifier(model_dir: str):
    """Load an ONNX classifier from a directory; None when unavailable.

    Requires the optional dependencies onnxruntime + tokenizers.
    """
    directory = Path(model_dir)
    model_file = directory / "model.onnx"
    tokenizer_file = directory / "tokenizer.json"
    config_file = directory / "config.json"
    if not (model_file.is_file() and tokenizer_file.is_file() and config_file.is_file()):
        return None
    try:
        from onnxruntime import InferenceSession  # noqa: PLC0415
        from tokenizers import Tokenizer  # noqa: PLC0415
    except ImportError:
        logger.warning("onnxruntime/tokenizers not installed; ML detector disabled")
        return None
    session = InferenceSession(str(model_file))
    tokenizer = Tokenizer.from_file(str(tokenizer_file))
    id2label: dict[str, str] = {}
    if config_file.is_file():
        cfg = json.loads(config_file.read_text(encoding="utf-8"))
        raw = cfg.get("id2label", {})
        id2label = {int(k): v for k, v in raw.items()} if isinstance(raw, dict) else {}
    return OnnxTextClassifier(session, tokenizer, id2label)


class MLInjectionInspector:
    """Verdict from a local ML classifier mapped to severity levels."""

    def __init__(
        self,
        classifier: Any,
        config: InjectionConfig,
        publish: Callable[[Event], None] | None = None,
    ):
        self._classifier = classifier
        self._config = config
        self._publish = publish or (lambda event: None)

    def process_request(
        self, payload: dict[str, Any], client: str | None = None
    ) -> None:
        if self._config.mode == "off":
            return
        threshold = self._config.ml.threshold
        findings: list[tuple[str, str]] = []
        for message in payload.get("messages", []):
            content = message.get("content")
            if not isinstance(content, str) or not content.strip():
                continue
            label, confidence = self._classifier.predict(content)
            if label != "benign" and confidence >= threshold:
                findings.append(
                    (f"ml:{label}", _severity_from_confidence(confidence))
                )
        handle_findings(findings, self._config, self._publish, "ml", client)

    def process_response(self, text: str, part: None) -> str:
        return text

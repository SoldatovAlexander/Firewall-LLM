# Firewall LLM Enterprise Modules

**COMMERCIAL SOFTWARE — commercial license from the copyright holder required.
Not covered by the FSL-1.1-MIT license of the fwllm core.**

Included modules:
- `egress_pools` — multi-pool egress proxies with rotation by request count,
  failure thresholds and cooldowns, per-adapter bindings.
- `ml_injection` — local ONNX prompt-injection classifier. Enable via
  `inspectors.injection.ml: {enabled: true, model_dir: /models/pi}`; the
  directory must contain `model.onnx`, `tokenizer.json`, `config.json`
  (with `id2label`). Requires optional deps: `pip install onnxruntime tokenizers`.
  Confidence maps to severity: >=0.9 critical, >=0.8 high, >=0.7 medium;
  verdicts feed the same block/log pipeline and `attack_detected` events.

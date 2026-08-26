# Inspectors

Chain: `injection (signatures)` → `ML` → `DLP`.

**Signatures:** `override_instructions` (critical), `jailbreak_persona` (high), `roleplay_probe` (medium). `block_severity_gte`.

**ML (enterprise):** `injection.ml: {enabled, model_dir, threshold}` — ONNX `model.onnx` + `tokenizer.json`, confidence → severity (≥0.9 critical, ≥0.8 high, ≥0.7 medium).

**DLP:** `mode: block/mask/log/off`, `restore_policy: mask/restore`, `profile: ru_152`. `sanitize_with_scope` → `vault` per request.

# Inspectors

Chain: `injection (signatures)` → `ML` → `DLP`.

**Signatures:** `override_instructions` (critical), `jailbreak_persona` (high), `roleplay_probe` (medium). `block_severity_gte`.

**ML (enterprise):** `injection.ml: {enabled, model_dir, threshold}` — ONNX `model.onnx` + `tokenizer.json`, confidence → severity (≥0.9 critical, ≥0.8 high, ≥0.7 medium).

**DLP:** `mode: block/mask/log/off`, `restore_policy: mask/restore`, `profile: ru_152`. `sanitize_with_scope` → `vault` per request.

## DLP parity: Python vs Rust (release 0.1.0)

Both branches: per-request vault, `block/mask/log/off` modes, `restore_policy: mask/restore`, streaming token reassembly (R13). **Detector coverage differs** — a known release limitation, not parity:

| Data type | Python (LightAnon `ru_152`) | Rust (regex) |
|---|---|---|
| EMAIL | ✅ | ✅ |
| PHONE (RU) | ✅ | ✅ |
| CARD (13–19 digits) | ✅ | ✅ |
| PASSPORT (RU) | ✅ | ❌ |
| SNILS | ✅ | ❌ |
| INN | ✅ | ❌ |
| PERSON (RU names) | ✅ | ❌ |
| ONLINE_ACCOUNT / PROFILE_URL / SOCIAL_HANDLE / USERNAME | ✅ | ❌ |

The Python side is defined by the LightAnon pin in `py/fwllm` (`ru_152` = 11 types). Extending the Rust side is out of 0.1.0 scope; until then only `injection`/policies apply to those types on the Rust gateway. Legal sufficiency of either side is not assessed by this release.

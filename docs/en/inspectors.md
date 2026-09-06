# Inspectors

Chain: `injection (signatures)` → `ML` → `DLP`.

**Signatures:** `override_instructions` (critical), `jailbreak_persona` (high), `roleplay_probe` (medium). `block_severity_gte`.

**ML (enterprise):** `injection.ml: {enabled, model_dir, threshold}` — ONNX `model.onnx` + `tokenizer.json`, confidence → severity (≥0.9 critical, ≥0.8 high, ≥0.7 medium).

**DLP:** `mode: block/mask/log/off`, `restore_policy: mask/restore`, `profile: ru_152`. `sanitize_with_scope` → `vault` per request.

## DLP parity: Python vs Rust (0.1.1 — detector parity)

Both branches: per-request vault, `block/mask/log/off` modes, `restore_policy: mask/restore`, streaming token reassembly (R13). Rust detectors mirror the LightAnon `ru_152` profile verbatim (patterns ported 1:1, same order); a shared 12-sample corpus is locked by both branches' tests (`test_dlp_parity_corpus_ru152`).

| Data type | Python (LightAnon `ru_152`) | Rust (regex) |
|---|---|---|
| EMAIL | ✅ | ✅ |
| PHONE (RU) | ✅ | ✅ |
| CARD (13–19 digits) | ✅ | ✅ |
| PASSPORT (RU) | ✅ | ✅ |
| SNILS | ✅ | ✅ |
| INN (10/12 digits) | ✅ | ✅ |
| PERSON (RU names) | ✅ | ✅ |
| ONLINE_ACCOUNT (RU+EN) | ✅ | ✅ |
| PROFILE_URL | ✅ | ✅ |
| SOCIAL_HANDLE | ✅ | ✅ |
| USERNAME (labelled) | ✅ | ✅ |

Known divergences (documented, erring toward over-mask): the regex crate has no look-around — PERSON's trailing `(?!\w)` and SOCIAL_HANDLE's leading `(?<![\w.%+-])` are dropped; the latter's safety rests on ordering (EMAIL runs first, `ivan@mail.ru` never fragments — covered by test). `dlp.profile` still does not switch detector sets in Rust. Legal sufficiency of either side is not assessed.

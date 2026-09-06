# Инспекторы

Цепочка: `injection (сигнатуры)` → `ML` → `DLP`.

**Сигнатуры:** `override_instructions` (critical) и т.д. `block_severity_gte`.

**ML (enterprise):** `injection.ml: {enabled, model_dir, threshold}` — ONNX.

**DLP:** `mode: block/mask/log/off`, `profile: ru_152`.

## DLP parity: Python vs Rust (релиз 0.1.0)

Обе ветки: vault на запрос, режимы `block/mask/log/off`, `restore_policy: mask/restore`, стриминг-реассембли токенов (R13). **Состав детекторов различается** — это известное ограничение релиза, а не паритет:

| Тип данных | Python (LightAnon `ru_152`) | Rust (regex) |
|---|---|---|
| EMAIL | ✅ | ✅ |
| PHONE (RU) | ✅ | ✅ |
| CARD (13–19 цифр) | ✅ | ✅ |
| PASSPORT (RU) | ✅ | ❌ |
| SNILS | ✅ | ❌ |
| INN | ✅ | ❌ |
| PERSON (RU ФИО) | ✅ | ❌ |
| ONLINE_ACCOUNT / PROFILE_URL / SOCIAL_HANDLE / USERNAME | ✅ | ❌ |

Состав Python-стороны задан пином LightAnon в `py/fwllm` (профиль `ru_152` = 11 типов). Расширение Rust-стороны — за скоупом 0.1.0; до него для этих типов на Rust-гейте действует только `injection`/политики. Юридическая достаточность ни одной из сторон этим релизом не оценивается.

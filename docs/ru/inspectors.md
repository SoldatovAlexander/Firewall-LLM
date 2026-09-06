# Инспекторы

Цепочка: `injection (сигнатуры)` → `ML` → `DLP`.

**Сигнатуры:** `override_instructions` (critical) и т.д. `block_severity_gte`.

**ML (enterprise):** `injection.ml: {enabled, model_dir, threshold}` — ONNX.

**DLP:** `mode: block/mask/log/off`, `profile: ru_152`.

## DLP parity: Python vs Rust (0.1.1 — паритет детекторов)

Обе ветки: vault на запрос, режимы `block/mask/log/off`, `restore_policy: mask/restore`, стриминг-реассембли токенов (R13). Состав детекторов Rust повторяет профиль LightAnon `ru_152` дословно (паттерны портированы 1:1, порядок тот же); общий корпус из 12 сэмплов зафиксирован тестами обеих веток (`test_dlp_parity_corpus_ru152`).

| Тип данных | Python (LightAnon `ru_152`) | Rust (regex) |
|---|---|---|
| EMAIL | ✅ | ✅ |
| PHONE (RU) | ✅ | ✅ |
| CARD (13–19 цифр) | ✅ | ✅ |
| PASSPORT (RU) | ✅ | ✅ |
| SNILS | ✅ | ✅ |
| INN (10/12 цифр) | ✅ | ✅ |
| PERSON (RU ФИО) | ✅ | ✅ |
| ONLINE_ACCOUNT (RU+EN) | ✅ | ✅ |
| PROFILE_URL | ✅ | ✅ |
| SOCIAL_HANDLE | ✅ | ✅ |
| USERNAME (labelled) | ✅ | ✅ |

Известные расхождения ( задокументированы, в сторону over-mask): regex-crate не умеет look-around — убран trailing `(?!\w)` у PERSON и leading `(?<![\w.%+-])` у SOCIAL_HANDLE; безопасность второго держится порядком (EMAIL идёт первым, `ivan@mail.ru` не фрагментируется — покрыто тестом). `dlp.profile` по-прежнему не переключает набор детекторов в Rust. Юридическая достаточность ни одной из сторон не оценивается.

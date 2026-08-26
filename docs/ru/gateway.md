# Шлюз

Единый `POST /v1/chat/completions` (OpenAI-совместимый), `GET /healthz`, `GET /metrics`, `GET /admin/audit`, `POST /admin/ingress/tokens`.

**Аутентификация:** `Authorization: Bearer <client-key>` из `clients` или `FWLLM_CLIENT_TOKENS` (`key:label,...`). Пустой `clients` → 401 всем.

**Ошибки:**
| Код | HTTP | `error.type` |
|-----|------|--------------|
| невалидное тело | 422 | `invalid_request_error` |
| нет/неверный ключ | 401 | `authentication_error` |
| инъекция/DLP/blocked_source | 403 | `permission_error` |
| квота | 429 | `rate_limit_error` |
| апстрим | 502 | `upstream_error` |

**Стриминг:** `stream: true` → `text/event-stream`.

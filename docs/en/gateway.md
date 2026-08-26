# Gateway

Unified `POST /v1/chat/completions` (OpenAI-compatible), `GET /healthz`, `GET /metrics`, `GET /admin/audit`, `POST /admin/ingress/tokens`.

**Auth:** `Authorization: Bearer <client-key>` from `clients` map or `FWLLM_CLIENT_TOKENS` env (`key:label,...`). Empty `clients` → 401 for all.

**Errors (contract `contracts/openapi.yaml`):**
| Code | HTTP | `error.type` |
|------|------|--------------|
| invalid body | 422 | `invalid_request_error` |
| missing/invalid key | 401 | `authentication_error` |
| injection/DLP/blocked_source | 403 | `permission_error` |
| quota | 429 | `rate_limit_error` |
| upstream | 502 | `upstream_error` |

**Streaming:** `stream: true` → `text/event-stream` with `data: {chunk}` and `data: [DONE]`.

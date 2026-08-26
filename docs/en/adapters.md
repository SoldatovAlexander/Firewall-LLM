# Adapters

One adapter per external LLM API. All speak `POST /chat/completions`.

| Adapter | `type` | Notes |
|---------|--------|-------|
| `openai_compat` | generic | any `base_url` |
| `openrouter` | `openrouter` | adds `HTTP-Referer` / `X-Title` |
| `ollama` | `ollama` | `http://host:11434/v1` |
| `tunnel` | `tunnel` | via `wss://:8443` agent, `agent_id` + `base_url` |

Registry builds from `config.providers` (`base_url`, `api_key_env`). Missing `base_url` → `ConfigError`.

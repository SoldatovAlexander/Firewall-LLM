# Адаптеры

Один адаптер на внешний LLM API. Все — `POST /chat/completions`.

| Адаптер | `type` | Примечания |
|---------|--------|------------|
| `openai_compat` | generic | любой `base_url` |
| `openrouter` | `openrouter` | добавляет `HTTP-Referer` |
| `ollama` | `ollama` | `http://host:11434/v1` |
| `tunnel` | `tunnel` | через `wss://:8443` агента |

Реестр строится из `config.providers`.

# Ingress-туннель

Self-signed TLS на `:8443`, `wss://gateway:8443/ingress`.

**Паринг:**
```bash
curl -H "Authorization: Bearer $CLIENT_KEY" -X POST http://gateway:8080/admin/ingress/tokens -d '{"agent_id":"llm-remote-01"}'
cargo run -p fwllm-agent -- --gateway-url wss://gateway:8443/ingress --token <token> --ca-cert ./certs/ca.crt
```

Агент маскирует заголовки и форвардит к `Destination` из пакета шлюза.

## Закалка туннеля (0.1.1)

- Очередь на агента bounded (16): переполнение отвечает `agent overloaded`, а не растит память.
- Heartbeat: gateway шлёт Ping каждые 30с, молчуны свыше 90с дропаются; disconnect чистит запись агента и туннель (следующий forward — сразу `no tunnel`, in-flight — `agent dropped`, а не 30с висения).
- Агент переподключается сам (backoff 1с→60с, `--max-retries N`, `0` — старое поведение выхода); каждый forward ограничен 120с.
- Покрыто: unit (unregister/sweep/bounded/heartbeat-флаг/backoff) + `real_tunnel_test` на настоящих бинарниках (включая reconnect).

## Остаточные ограничения

- Агент форвардит последовательно (один запрос за раз).
- `TunnelProvider` не реализует `chat_stream`: стриминг через туннель недоступен — осознанно, явная ошибка `streaming unsupported` (фреймы потока — за скоупом 0.1.1).

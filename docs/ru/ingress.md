# Ingress-туннель

Self-signed TLS на `:8443`, `wss://gateway:8443/ingress`.

**Паринг:**
```bash
curl -H "Authorization: Bearer $CLIENT_KEY" -X POST http://gateway:8080/admin/ingress/tokens -d '{"agent_id":"llm-remote-01"}'
cargo run -p fwllm-agent -- --gateway-url wss://gateway:8443/ingress --token <token> --ca-cert ./certs/ca.crt
```

Агент маскирует заголовки и форвардит к `Destination` из пакета шлюза.

## Известные ограничения туннеля (релиз 0.1.0)

- Канал реестра и карта pending-запросов без лимитов и очистки по deadline; запись агента при disconnect может оставаться висеть.
- Агент форвардит HTTP последовательно, без общего request timeout и reconnect-loop с backoff — падение агента требует ручного перезапуска.
- `TunnelProvider` не реализует `chat_stream`: стриминг через туннель недоступен (вернётся `streaming unsupported`).
- Heartbeat соединения нет — «молчаливый» обрыв обнаруживается только на следующем запросе.

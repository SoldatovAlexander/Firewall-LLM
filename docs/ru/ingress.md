# Ingress-туннель

Self-signed TLS на `:8443`, `wss://gateway:8443/ingress`.

**Паринг:**
```bash
curl -H "Authorization: Bearer $CLIENT_KEY" -X POST http://gateway:8080/admin/ingress/tokens -d '{"agent_id":"llm-remote-01"}'
cargo run -p fwllm-agent -- --gateway-url wss://gateway:8443/ingress --token <token> --ca-cert ./certs/ca.crt
```

Агент маскирует заголовки и форвардит к `Destination` из пакета шлюза.

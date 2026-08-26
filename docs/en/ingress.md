# Ingress Tunnel

Self-signed TLS on `:8443` (generated via `rcgen` to `./certs/`), `wss://gateway:8443/ingress`.

**Pairing:**
```bash
curl -H "Authorization: Bearer $CLIENT_KEY" -X POST http://gateway:8080/admin/ingress/tokens -d '{"agent_id":"llm-remote-01"}'
# → {token, expires_at}

cargo run -p fwllm-agent -- --gateway-url wss://gateway:8443/ingress --token <token> --ca-cert ./certs/ca.crt
```

Agent masks `Via/X-Forwarded-*` and forwards `{id,method,url,headers,body}` → `Destination` (LLM API URL from gateway packet).

`egress.mode: tunnel` provider uses `agent_id` + `base_url` via `IngressRegistry` channel.

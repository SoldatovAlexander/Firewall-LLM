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

## Known tunnel limitations (release 0.1.0)

- Registry channel and pending-request map have no limits or deadline cleanup; an agent record may linger after disconnect.
- The agent forwards HTTP sequentially, with no global request timeout and no reconnect loop with backoff — a crashed agent needs a manual restart.
- `TunnelProvider` does not implement `chat_stream`: streaming over the tunnel is unavailable (returns `streaming unsupported`).
- No connection heartbeat — a silent drop is only noticed on the next request.

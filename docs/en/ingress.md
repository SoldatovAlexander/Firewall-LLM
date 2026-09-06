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

## Tunnel hardening (0.1.1)

- Bounded per-agent queue (16): overflow fails fast with `agent overloaded` instead of growing memory.
- Heartbeat: the gateway pings every 30s, agents silent over 90s are dropped; disconnect cleans the agent record and tunnel (next forward fails at once with `no tunnel`, in-flight ones resolve as `agent dropped`, not a 30s hang).
- The agent reconnects on its own (backoff 1s→60s, `--max-retries N`, `0` keeps the old exit behavior); every forward is bounded by 120s.
- Covered by unit tests (unregister/sweep/bounded/heartbeat-flag/backoff) + `real_tunnel_test` on real binaries (including reconnect).

## Residual limitations

- The agent forwards sequentially (one request at a time).
- `TunnelProvider` does not implement `chat_stream`: streaming over the tunnel is unavailable — deliberate, explicit `streaming unsupported` error (stream frames are out of 0.1.1 scope).

# Egress

```yaml
egress:
  mode: direct            # open core
  # mode: single_proxy    # one proxy for all adapters
  # proxy_url: http://proxy:8080
  # mode: pools           # enterprise
  # pools: { main: { proxies: ["http://p1:8080"], rotation: round_robin } }
  # bindings: { openrouter: main }
```

`tunnel` provider uses `wss://:8443` agent instead of HTTP proxy.

## SOCKS and live egress (0.1.1)

Both branches speak `socks5h://` (`httpx[socks]`, reqwest `socks`; without them single_proxy on a SOCKS URL crashed the Python gateway at startup). Verified live: chats via `single_proxy` to OpenRouter return 200 on both branches.

Public proxies die within hours (observed) and expose traffic to foreign exit nodes — production needs its own proxy. Selection method: the `/api/v1/auth/key` oracle with a dummy key (401 = exit IP allowed, 403 = blocked; the real key never leaves the perimeter).

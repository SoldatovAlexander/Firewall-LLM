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

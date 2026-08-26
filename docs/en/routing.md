# Routing

```yaml
routing:
  state_store: redis          # memory | redis
  default_chain: [primary, backup]
  model_mapping:
    gpt-4o:
      primary: gpt-4o-2024
  rules:
    - name: budget
      when: { provider: primary, provider_tokens_today: {gt: 50} }
      action: { next_in_chain: true }
  attack_failover:
    enabled: true
    count: 5
    window_seconds: 300
    min_severity: high
    switch_to: backup
    block_source: true
```

`routed_from` set when remapped. `state_store: redis` survives restarts.

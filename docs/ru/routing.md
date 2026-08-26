# Маршрутизация

```yaml
routing:
  state_store: redis
  default_chain: [primary, backup]
  attack_failover:
    enabled: true
    count: 5
    switch_to: backup
```

`state_store: redis` переживает рестарт. `routed_from` при ремапе.

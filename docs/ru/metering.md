# Учёт

Дневные бакеты в Redis: `fwllm:c:tokens:{client}:{day}` (TTL 48ч).

```yaml
quotas:
  client_tokens_per_day: 50000
```

`check_client` → 429. При падении Redis — fail-open.

# Metering

Daily buckets in Redis: `fwllm:c:tokens:{client}:{day}` (TTL 48h).

```yaml
quotas:
  client_tokens_per_day: 50000
  client_requests_per_day: 1000
```

`check_client` before provider → 429. Fail-open if Redis down.

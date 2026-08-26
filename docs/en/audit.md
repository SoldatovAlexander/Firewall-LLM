# Audit

SQLite `audit.db` (`/data/audit.db`).

```yaml
audit:
  enabled: true
  db_path: /data/audit.db
  dlp_redact: true
```

`GET /admin/audit?client=&code=&limit=` (bearer) → `{total, records[]}`. PII redacted (EMAIL→[EMAIL], CARD, PHONE).

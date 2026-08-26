# Аудит

SQLite `audit.db`.

```yaml
audit: { enabled, db_path, dlp_redact }
```

`GET /admin/audit` → `{total, records[]}`. PII вырезается.

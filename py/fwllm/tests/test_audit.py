"""Audit log tests: storage, PII redaction, search filters."""

from fwllm.audit import AuditConfig, AuditLog


def _record(**overrides):
    base = {
        "client": "alice",
        "provider": "openrouter",
        "model": "gpt-4o",
        "code": "ok",
        "prompt_tokens": 10,
        "completion_tokens": 5,
        "messages": [{"role": "user", "content": "hello world"}],
        "response_text": "Hi there!",
    }
    base.update(overrides)
    return base


def test_write_and_search_roundtrip(tmp_path):
    log = AuditLog(AuditConfig(db_path=str(tmp_path / "audit.db")))
    log.write(**_record())
    rows = log.search(limit=10)
    assert len(rows) == 1
    assert rows[0]["client"] == "alice"
    assert rows[0]["code"] == "ok"
    assert rows[0]["response_text"] == "Hi there!"


def test_pii_is_redacted_before_storage(tmp_path):
    log = AuditLog(AuditConfig(db_path=str(tmp_path / "audit.db"), dlp_redact=True))
    log.write(
        **_record(
            messages=[{"role": "user", "content": "my email is ivan@mail.ru"}],
            response_text="contact ivan@mail.ru later",
        )
    )
    row = log.search()[0]
    stored = str(row["messages"]) + row["response_text"]
    assert "ivan@mail.ru" not in stored


def test_redaction_can_be_disabled(tmp_path):
    log = AuditLog(AuditConfig(db_path=str(tmp_path / "audit.db"), dlp_redact=False))
    log.write(**_record(messages=[{"role": "user", "content": "ivan@mail.ru"}]))
    assert "ivan@mail.ru" in str(log.search()[0]["messages"])


def test_search_filters_by_client_and_code(tmp_path):
    log = AuditLog(AuditConfig(db_path=str(tmp_path / "audit.db")))
    log.write(**_record())
    log.write(**_record(client="bob", code="blocked"))
    log.write(**_record(code="upstream_error"))
    assert len(log.search()) == 3
    assert all(r["client"] == "bob" for r in log.search(client="bob"))
    codes = {r["code"] for r in log.search(code="blocked")}
    assert codes == {"blocked"}
    assert len(log.search(client="bob", code="blocked")) == 1


def test_limit_orders_newest_first(tmp_path):
    import time as time_mod

    log = AuditLog(AuditConfig(db_path=str(tmp_path / "audit.db")))
    for i in range(5):
        log.write(**_record(client=f"c{i}"))
        time_mod.sleep(0.01)
    rows = log.search()
    assert [r["client"] for r in rows] == ["c4", "c3", "c2", "c1", "c0"]

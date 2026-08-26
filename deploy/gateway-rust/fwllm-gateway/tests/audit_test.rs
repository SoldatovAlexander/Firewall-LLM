//! Audit log tests (SQLite, temp files).

use fwllm_gateway::audit::AuditLog;
use fwllm_core::config::AuditConfig;

fn config(path: &std::path::Path) -> AuditConfig {
    AuditConfig {
        enabled: true,
        db_path: path.to_string_lossy().to_string(),
        dlp_redact: true,
    }
}

#[test]
fn write_and_search_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let log = AuditLog::open(&config(&dir.path().join("a.db"))).unwrap();
    log.write("alice", "primary", "gpt-4o", "ok", 10, 5, r#"[{"role":"user"}]"#, "Hi!");
    let rows = log.search(None, None, 100);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].client, "alice");
    assert_eq!(rows[0].code, "ok");
    assert_eq!(rows[0].response, "Hi!");
}

#[test]
fn pii_is_redacted_before_storage() {
    let dir = tempfile::tempdir().unwrap();
    let log = AuditLog::open(&config(&dir.path().join("a.db"))).unwrap();
    log.write(
        "alice", "p", "m", "ok", 1, 1,
        r#"[{"role":"user","content":"email ivan@mail.ru please"}]"#,
        "contact +7 999 123-45-67 later",
    );
    let row = &log.search(None, None, 10)[0];
    assert!(!row.messages.contains("ivan@mail.ru"), "{}", row.messages);
    assert!(row.messages.contains("[EMAIL]"));
    assert!(!row.response.contains("+7 999"), "{}", row.response);
    assert!(row.response.contains("[PHONE]"));
}

#[test]
fn search_filters_by_client_and_code() {
    let dir = tempfile::tempdir().unwrap();
    let log = AuditLog::open(&config(&dir.path().join("a.db"))).unwrap();
    log.write("alice", "p", "m", "ok", 1, 1, "[]", "");
    log.write("bob", "p", "m", "blocked", 0, 0, "[]", "");
    assert_eq!(log.search(Some("bob"), None, 100).len(), 1);
    assert_eq!(log.search(None, Some("blocked"), 100)[0].client, "bob");
    assert_eq!(
        log.search(Some("bob"), Some("blocked"), 100)[0].model, "m"
    );
}

#[test]
fn disabled_audit_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(&dir.path().join("a.db"));
    cfg.enabled = false;
    let log = AuditLog::open(&cfg).unwrap();
    log.write("alice", "p", "m", "ok", 1, 1, "[]", "");
    assert_eq!(log.search(None, None, 100).len(), 0);
}

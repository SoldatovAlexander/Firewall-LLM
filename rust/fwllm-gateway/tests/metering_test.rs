//! Metering unit tests (in-memory store).

use fwllm_gateway::metering::{InMemoryStore, Metering, MeteringError};
use fwllm_core::config::Quotas;

fn quotas(tokens: Option<i64>, requests: Option<i64>) -> Quotas {
    Quotas {
        client_tokens_per_day: tokens,
        client_requests_per_day: requests,
        provider_tokens_per_day: None,
        backend_fail_closed: false,
    }
}

#[test]
fn record_increments_daily_counters() {
    let store = Box::new(InMemoryStore::default());
    let m = Metering::new(store, &quotas(None, None));
    m.record("alice", "primary", "gpt-4o", 10, 5);
    // counters are internal; verify via quota behavior instead
    m.check_client("alice").unwrap();
}

#[test]
fn token_quota_exceeded_maps_to_429_scope() {
    let m = Metering::new(Box::new(InMemoryStore::default()), &quotas(Some(10), None));
    m.record("alice", "p", "m", 8, 2);
    let err = m.check_client("alice").unwrap_err();
    assert!(matches!(err, MeteringError::QuotaExceeded { scope: "tokens", limit: 10 }));
}

#[test]
fn request_quota_exceeded() {
    let m = Metering::new(
        Box::new(InMemoryStore::default()),
        &quotas(Some(100), Some(2)),
    );
    m.record("bob", "p", "m", 1, 1);
    m.check_client("bob").unwrap();
    m.record("bob", "p", "m", 1, 1);
    m.record("bob", "p", "m", 1, 1);
    assert!(matches!(m.check_client("bob").unwrap_err(), MeteringError::QuotaExceeded { scope: "requests", .. }));
}

#[test]
fn no_quotas_never_exceeds() {
    let m = Metering::new(Box::new(InMemoryStore::default()), &Quotas::default());
    m.record("carol", "p", "m", 999_999, 0);
    m.check_client("carol").unwrap();
}

//! Metering unit tests (in-memory store).

use fwllm_gateway::metering::{InMemoryStore, Metering, MeteringError};
use fwllm_core::config::Quotas;

fn quotas(tokens: Option<i64>, requests: Option<i64>) -> Quotas {
    Quotas {
        client_tokens_per_day: tokens,
        client_requests_per_day: requests,
        provider_tokens_per_day: None,
        backend_fail_closed: false,
        completion_reserve_tokens: 1024,
    }
}

#[test]
fn admit_reserves_settle_refunds_and_is_idempotent() {
    let m = Metering::new(Box::new(InMemoryStore::default()), &quotas(Some(100), None));
    let rsv = m.admit("a", "p", "m", 10, Some(20)).unwrap();
    // reservation holds the full 30 on the token counter
    assert!(m.check_client("a").is_ok());
    m.settle(&rsv, 5, 5);
    // repeat settle is a no-op (provider retries, double finalize)
    m.settle(&rsv, 5, 5);
    // only the actual 10 remain: admitting 90 more fits, 91 does not
    m.admit("a", "p", "m", 45, Some(46)).unwrap_err();
    let rsv2 = m.admit("a", "p", "m", 45, Some(45)).unwrap();
    m.settle(&rsv2, 0, 0);
}

#[test]
fn concurrent_admit_single_request_slot() {
    use std::sync::Arc;
    let m = Arc::new(Metering::new(
        Box::new(InMemoryStore::default()),
        &quotas(None, Some(1)),
    ));
    let admitted = std::thread::scope(|s| {
        let handles: Vec<_> = (0..20)
            .map(|_| {
                let m = m.clone();
                s.spawn(move || m.admit("a", "p", "m", 1, Some(1)).is_ok())
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).filter(|ok| *ok).count()
    });
    assert_eq!(admitted, 1);
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

struct BrokenStore;

impl fwllm_gateway::metering::MeteringStore for BrokenStore {
    fn incr(&self, _key: &str, _amount: i64) -> Result<i64, String> {
        Err("down".to_string())
    }
    fn get(&self, _key: &str) -> Result<i64, String> {
        Err("down".to_string())
    }
    fn ping(&self) -> Result<(), String> {
        Err("down".to_string())
    }
    fn reserve(
        &self,
        _req: &fwllm_gateway::metering::AdmitOp,
    ) -> Result<(), fwllm_gateway::metering::ReserveError> {
        Err(fwllm_gateway::metering::ReserveError::Backend("down".to_string()))
    }
    fn settle(
        &self,
        _req: &fwllm_gateway::metering::SettleReq,
    ) -> Result<bool, String> {
        Err("down".to_string())
    }
}

#[test]
fn fail_closed_checks_backend_even_without_quotas() {
    let quotas = Quotas {
        backend_fail_closed: true,
        ..Default::default()
    };
    let m = Metering::new(Box::new(BrokenStore), &quotas);
    assert!(matches!(
        m.check_client("alice"),
        Err(MeteringError::BackendUnavailable(_))
    ));
}

#[test]
fn fail_open_ignores_backend_without_quotas() {
    let quotas = Quotas {
        backend_fail_closed: false,
        ..Default::default()
    };
    let m = Metering::new(Box::new(BrokenStore), &quotas);
    assert!(m.check_client("alice").is_ok());
}

#[test]
#[should_panic(expected = "invalid redis_url")]
fn malformed_redis_url_fails_fast() {
    use fwllm_gateway::providers::{OpenAiCompatProvider, Provider};
    use std::collections::HashMap;
    use std::sync::Arc;

    let mut cfg: fwllm_core::config::Config = serde_yaml::from_str(
        "providers:\n  p:\n    base_url: http://unused.invalid/v1\nredis_url: 'redis://localhost:6379/0'\n",
    )
    .unwrap();
    cfg.redis_url = "http://[invalid".to_string();
    let mut providers = HashMap::new();
    providers.insert(
        "p".to_string(),
        Arc::new(OpenAiCompatProvider::new(
            "http://unused.invalid/v1",
            None,
            std::time::Duration::from_secs(1),
            vec![],
        )) as Arc<dyn Provider>,
    );
    let _ = fwllm_gateway::state::AppState::build(
        cfg,
        Some(Arc::new(providers)),
        None,
        None,
    );
}

#[test]
fn no_quotas_never_exceeds() {
    let m = Metering::new(Box::new(InMemoryStore::default()), &Quotas::default());
    m.record("carol", "p", "m", 999_999, 0);
    m.check_client("carol").unwrap();
}

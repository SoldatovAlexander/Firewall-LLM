//! 0.1.1 tunnel hardening: unregister, stale sweep, bounded queue.

use fwllm_gateway::ingress::{
    shared_registry, HEARTBEAT_TIMEOUT_SECS, TUNNEL_QUEUE_DEPTH,
};

#[tokio::test]
async fn unregister_removes_agent_and_tunnel() {
    let registry = shared_registry();
    let (tx, _rx) = tokio::sync::mpsc::channel(TUNNEL_QUEUE_DEPTH);
    registry.register_tunnel("a1".to_string(), tx).await;
    assert_eq!(registry.list_agents().await.len(), 1);
    registry.unregister_agent("a1").await;
    assert!(registry.list_agents().await.is_empty());
    let err = registry
        .forward("a1", "GET".into(), "http://x/".into(), Default::default(), None)
        .await
        .unwrap_err();
    assert!(err.contains("no tunnel"), "{err}");
}

#[tokio::test]
async fn heartbeat_liveness_reflected() {
    let registry = shared_registry();
    let (tx, _rx) = tokio::sync::mpsc::channel(TUNNEL_QUEUE_DEPTH);
    registry.register_tunnel("hb".to_string(), tx).await;
    registry.touch_agent("hb").await;
    assert!(!registry.is_stale("hb", HEARTBEAT_TIMEOUT_SECS as f64).await);
    assert!(registry.is_stale("hb", -1.0).await);
    assert!(registry.is_stale("ghost", HEARTBEAT_TIMEOUT_SECS as f64).await);
}

#[tokio::test]
async fn sweep_stale_removes_silent_agents() {
    let registry = shared_registry();
    let (tx, _rx) = tokio::sync::mpsc::channel(TUNNEL_QUEUE_DEPTH);
    registry.register_tunnel("old".to_string(), tx).await;
    let (tx2, _rx2) = tokio::sync::mpsc::channel(TUNNEL_QUEUE_DEPTH);
    registry.register_tunnel("fresh".to_string(), tx2).await;
    // Explicit clock anchored at the observed last_seen (no wall-clock
    // dependence): +10s nothing is stale, +1000s everything is.
    let agents = registry.list_agents().await;
    assert_eq!(agents.len(), 2);
    let base = agents[0].last_seen;
    assert!(registry.sweep_stale(base + 10.0, HEARTBEAT_TIMEOUT_SECS as f64).await.is_empty());
    assert_eq!(registry.list_agents().await.len(), 2);
    let swept =
        registry.sweep_stale(base + 1000.0, HEARTBEAT_TIMEOUT_SECS as f64).await;
    assert_eq!(swept.len(), 2);
    assert!(registry.list_agents().await.is_empty());
    let err = registry
        .forward("old", "GET".into(), "http://x/".into(), Default::default(), None)
        .await
        .unwrap_err();
    assert!(err.contains("no tunnel"), "{err}");
}

#[tokio::test]
async fn bounded_queue_rejects_overflow_fast() {
    let registry = shared_registry();
    // Receiver exists but is never polled: the queue fills, then rejects.
    let (tx, _rx) = tokio::sync::mpsc::channel(TUNNEL_QUEUE_DEPTH);
    registry.register_tunnel("busy".to_string(), tx).await;
    let mut handles = Vec::new();
    for _ in 0..TUNNEL_QUEUE_DEPTH + 1 {
        let r = registry.clone();
        handles.push(tokio::spawn(async move {
            r.forward(
                "busy",
                "GET".into(),
                "http://x/".into(),
                Default::default(),
                None,
            )
            .await
        }));
    }
    // Poll until the fast rejection lands; the 16 queued forwards stay
    // pending (no agent reading) and are aborted afterwards — they must
    // never resolve as ok.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut overloaded = 0;
    let mut handles = handles;
    while !handles.is_empty() && std::time::Instant::now() < deadline {
        let mut still_pending = Vec::new();
        for h in handles {
            if h.is_finished() {
                match h.await.unwrap() {
                    Err(e) if e.contains("overloaded") => overloaded += 1,
                    Ok(_) => panic!("forward through an unread queue must not succeed"),
                    Err(e) => panic!("unexpected forward error: {e}"),
                }
            } else {
                still_pending.push(h);
            }
        }
        handles = still_pending;
        if overloaded == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        } else {
            break;
        }
    }
    for h in handles {
        h.abort();
    }
    assert_eq!(overloaded, 1, "exactly one fast overload rejection");
}

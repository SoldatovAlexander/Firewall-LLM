//! R02: egress proxy must be honored, never silently bypassed.

use fwllm_core::config::EgressConfig;
use fwllm_gateway::providers::{build_client, OpenAiCompatProvider, Provider};
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

/// Minimal counting HTTP proxy: answers absolute-form requests with canned body.
async fn spawn_counting_proxy(body: &'static str) -> (String, Arc<AtomicUsize>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let hits = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits_clone = hits.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let hits = hits_clone.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 65536];
                // read headers
                let mut head = Vec::new();
                loop {
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    head.extend_from_slice(&buf[..n]);
                    if head.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                // consume body if any
                let head_str = String::from_utf8_lossy(&head);
                let len = head_str
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("content-length:")
                            .or_else(|| l.strip_prefix("Content-Length:"))
                            .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                    })
                    .unwrap_or(0);
                let body_start = head
                    .windows(4)
                    .position(|w| w == b"\r\n\r\n")
                    .map(|p| p + 4)
                    .unwrap_or(head.len());
                let mut already = head.len().saturating_sub(body_start);
                while already < len {
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    already += n;
                }
                hits.fetch_add(1, Ordering::SeqCst);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });
    (format!("http://{addr}"), hits)
}

fn egress(mode: &str, proxy_url: Option<&str>) -> EgressConfig {
    EgressConfig {
        mode: mode.to_string(),
        proxy_url: proxy_url.map(str::to_string),
    }
}

#[test]
fn direct_mode_builds_client() {
    let client =
        build_client(&egress("direct", None), Duration::from_secs(5)).unwrap();
    let _ = client;
}

#[test]
fn single_proxy_missing_url_rejected() {
    let err = build_client(&egress("single_proxy", None), Duration::from_secs(5))
        .unwrap_err();
    assert!(err.contains("proxy_url"), "{err}");
}

#[test]
fn single_proxy_invalid_url_rejected() {
    let err = build_client(
        &egress("single_proxy", Some("http://[invalid")),
        Duration::from_secs(5),
    )
    .unwrap_err();
    assert!(!err.is_empty());
}

#[test]
fn unsupported_mode_rejected() {
    let err = build_client(
        &egress("pools", Some("http://127.0.0.1:8080")),
        Duration::from_secs(5),
    )
    .unwrap_err();
    assert!(err.contains("pools"), "{err}");
}

#[tokio::test]
async fn proxy_mode_traffic_goes_through_proxy() {
    let canned = r#"{"id":"via-proxy","object":"chat.completion"}"#;
    let (proxy_url, hits) = spawn_counting_proxy(canned).await;
    let client = build_client(
        &egress("single_proxy", Some(&proxy_url)),
        Duration::from_secs(5),
    )
    .unwrap();
    let provider = OpenAiCompatProvider::with_client(
        "http://upstream.invalid/v1",
        None,
        vec![],
        client,
    );
    let res = provider
        .chat(json!({"model": "m", "messages": []}))
        .await
        .unwrap();
    assert_eq!(res["id"], "via-proxy");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn proxy_down_means_error_not_direct_fallback() {
    // unroutable proxy port: connection refused must surface, never bypass
    let client = build_client(
        &egress("single_proxy", Some("http://127.0.0.1:1")),
        Duration::from_secs(5),
    )
    .unwrap();
    let provider = OpenAiCompatProvider::with_client(
        "http://upstream.invalid/v1",
        None,
        vec![],
        client,
    );
    let res = provider
        .chat(json!({"model": "m", "messages": []}))
        .await;
    assert!(res.is_err(), "proxy failure must not fall back to direct");
}

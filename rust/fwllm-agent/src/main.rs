use clap::Parser;
use http::HeaderMap;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::http::Request as WsRequest;

#[derive(Parser, Debug)]
#[command(name = "fwllm-agent")]
struct Args {
    /// Gateway wss URL, e.g. wss://fwllm.internal:8443/ingress
    #[arg(long, env = "FWLLM_GATEWAY_URL")]
    gateway_url: String,

    /// Bearer token issued via POST /admin/ingress/tokens
    #[arg(long, env = "FWLLM_TOKEN")]
    token: String,

    /// Path to CA cert for self-signed gateway (optional, skips verification if missing and --insecure)
    #[arg(long, env = "FWLLM_CA_CERT")]
    ca_cert: Option<String>,

    /// Skip TLS verification (for dev with self-signed without CA)
    #[arg(long, default_value_t = false)]
    insecure: bool,
}

/// Mask environment-leaking headers before forwarding.
pub fn mask_headers(headers: &mut http::HeaderMap) {
    for name in [
        "via",
        "x-forwarded-for",
        "x-forwarded-proto",
        "x-forwarded-host",
        "x-real-ip",
        "x-forwarded-port",
        "cf-connecting-ip",
        "cf-ray",
        "server",
        "x-powered-by",
    ] {
        headers.remove(name);
    }
    if !headers.contains_key("user-agent") {
        headers.insert(
            http::header::USER_AGENT,
            "Firewall-LLM-Agent/0.1".parse().unwrap(),
        );
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();
    tracing::info!("connecting to {}", args.gateway_url);
    let request = WsRequest::builder()
        .uri(&args.gateway_url)
        .header("Authorization", format!("Bearer {}", args.token))
        .body(())?;
    // TODO: picks up ca_cert / insecure via custom connector
    let (mut ws, _) = tokio_tungstenite::connect_async(request).await?;
    tracing::info!("tunnel established");
    while let Some(msg) = ws.next().await {
        let msg = msg?;
        if msg.is_text() {
            let text = msg.to_text()?;
            // expected frame: {id, method, url, headers, body}
            if let Ok(mut frame) = serde_json::from_str::<serde_json::Value>(text) {
                if let Some(obj) = frame.as_object_mut() {
                    // mask headers in place if present
                    if let Some(headers) = obj.get_mut("headers") {
                        if let Some(map) = headers.as_object_mut() {
                            let mut hm = http::HeaderMap::new();
                            for (k, v) in map.iter() {
                                if let (Ok(name), Ok(val)) = (
                                    k.parse::<http::HeaderName>(),
                                    v.as_str().unwrap_or("").parse::<http::HeaderValue>(),
                                ) {
                                    hm.insert(name, val);
                                }
                            }
                            mask_headers(&mut hm);
                            *map = hm
                                .iter()
                                .map(|(k, v)| {
                                    (
                                        k.to_string(),
                                        serde_json::Value::String(
                                            v.to_str().unwrap_or("").to_string(),
                                        ),
                                    )
                                })
                                .collect();
                        }
                    }
                    // forward to destination
                    let url = obj.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let method = obj.get("method").and_then(|v| v.as_str()).unwrap_or("GET").to_string();
                    let id = obj.get("id").cloned().unwrap_or(serde_json::Value::String("0".into()));
                    let resp = forward(&method, &url, &frame).await;
                    let reply = serde_json::json!({
                        "id": id,
                        "status": resp.status,
                        "headers": resp.headers,
                        "body": resp.body,
                    });
                    ws.send(tokio_tungstenite::tungstenite::Message::Text(reply.to_string().into())).await?;
                }
            }
        }
    }
    Ok(())
}

struct ForwardResp { status: u16, headers: serde_json::Value, body: String }

async fn forward(_method: &str, url: &str, _frame: &serde_json::Value) -> ForwardResp {
    // TODO: actual reqwest forwarding with body streaming
    if url.is_empty() {
        return ForwardResp { status: 502, headers: serde_json::json!({}), body: "missing url".into() };
    }
    // placeholder echo for now (TDD Green will replace)
    ForwardResp { status: 200, headers: serde_json::json!({}), body: String::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_removes_via_and_forwarded() {
        let mut hm = http::HeaderMap::new();
        hm.insert("via", "1.1 proxy".parse().unwrap());
        hm.insert("x-forwarded-for", "1.1.1.1".parse().unwrap());
        hm.insert("x-real-ip", "2.2.2.2".parse().unwrap());
        hm.insert("authorization", "Bearer token".parse().unwrap());
        mask_headers(&mut hm);
        assert!(!hm.contains_key("via"));
        assert!(!hm.contains_key("x-forwarded-for"));
        assert!(!hm.contains_key("x-real-ip"));
        assert!(hm.contains_key("authorization"));
        assert!(hm.contains_key("user-agent"));
    }

    #[test]
    fn mask_preserves_content_type() {
        let mut hm = http::HeaderMap::new();
        hm.insert("content-type", "application/json".parse().unwrap());
        mask_headers(&mut hm);
        assert!(hm.contains_key("content-type"));
    }
}

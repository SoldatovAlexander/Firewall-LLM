use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use http::HeaderMap;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::Request as WsRequest;

/// R09: build the WebSocket handshake request via IntoClientRequest so the
/// mandatory upgrade headers (Sec-WebSocket-Key, Upgrade, Connection,
/// Sec-WebSocket-Version) are generated; then attach agent auth.
pub fn build_request(url: &str, token: &str) -> anyhow::Result<WsRequest<()>> {
    let mut request = url.into_client_request()?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {token}").parse()?,
    );
    Ok(request)
}

#[derive(Parser, Debug)]
#[command(name = "fwllm-agent")]
struct Args {
    #[arg(long, env = "FWLLM_GATEWAY_URL")]
    gateway_url: String,
    #[arg(long, env = "FWLLM_TOKEN")]
    token: String,
    #[arg(long, env = "FWLLM_CA_CERT")]
    ca_cert: Option<String>,
    #[arg(long, default_value_t = false)]
    insecure: bool,
    /// 0.1.1: stop after N failed attempts instead of retrying forever.
    /// 0 keeps the old exit-on-first-failure behavior (scripts/tests).
    #[arg(long)]
    max_retries: Option<u32>,
}

pub fn mask_headers(headers: &mut HeaderMap) {
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

#[derive(Debug)]
struct NoCertificateVerification;
impl rustls::client::danger::ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA1,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
        ]
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // R09: rustls 0.23 has no auto-selected process provider; the real
    // binary panicked before even opening the handshake without this.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("ring provider install");
    tracing_subscriber::fmt::init();
    let args = Args::parse();
    tracing::info!("connecting to {}", args.gateway_url);

    let mut client_builder = reqwest::Client::builder()
        // 0.1.1: every forwarded request is bounded — a hung upstream must
        // not wedge the agent's sequential forward loop forever.
        .timeout(std::time::Duration::from_secs(120));
    if args.insecure {
        client_builder = client_builder.danger_accept_invalid_certs(true);
    } else if let Some(ca_path) = &args.ca_cert {
        let ca_pem = std::fs::read(ca_path)?;
        let cert = reqwest::Certificate::from_pem(&ca_pem)?;
        client_builder = client_builder.add_root_certificate(cert);
    }
    let http_client = client_builder.build()?;

    let ws_config: Option<std::sync::Arc<rustls::ClientConfig>> = if args.insecure {
        Some(std::sync::Arc::new(
            rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(std::sync::Arc::new(NoCertificateVerification))
                .with_no_client_auth(),
        ))
    } else if let Some(ca_path) = &args.ca_cert {
        let ca_pem = std::fs::read(ca_path)?;
        let mut root_store = rustls::RootCertStore::empty();
        let mut reader = std::io::BufReader::new(ca_pem.as_slice());
        for cert in rustls_pemfile::certs(&mut reader) {
            root_store.add(cert?)?;
        }
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        Some(std::sync::Arc::new(
            rustls::ClientConfig::builder()
                .with_root_certificates(root_store)
                .with_no_client_auth(),
        ))
    } else {
        None
    };

    let connector = ws_config.map(tokio_tungstenite::Connector::Rustls);
    // 0.1.1: reconnect loop — a dropped tunnel re-establishes with backoff
    // instead of exiting (exit code used to be the only signal).
    let mut attempt: u32 = 0;
    loop {
        let request = build_request(&args.gateway_url, &args.token)?;
        let outcome = serve_once(request, &http_client, connector.clone()).await;
        attempt = attempt.saturating_add(1);
        if let Some(max) = args.max_retries {
            if attempt > max {
                // Old behavior for scripts/tests: surface the last result.
                return match outcome {
                    Ok(()) => Ok(()),
                    Err(e) => Err(e),
                };
            }
        }
        match &outcome {
            Ok(()) => tracing::info!("tunnel closed by gateway, reconnecting"),
            Err(e) => tracing::warn!("tunnel error: {e:#}, reconnecting"),
        }
        let delay = backoff_delay(attempt);
        tracing::info!("reconnect attempt {attempt} in {delay:?}");
        tokio::time::sleep(delay).await;
    }
}

/// 0.1.1: exponential backoff 1s, 2s, 4s … capped at 60s. Pure for tests.
pub fn backoff_delay(attempt: u32) -> std::time::Duration {
    let secs = 1u64.checked_shl(attempt.saturating_sub(1).min(6)).unwrap_or(64);
    std::time::Duration::from_secs(secs.min(60))
}

async fn serve_once(
    request: WsRequest<()>,
    http_client: &reqwest::Client,
    connector: Option<tokio_tungstenite::Connector>,
) -> anyhow::Result<()> {
    use tokio_tungstenite::tungstenite::Message;
    let (mut ws, _) =
        tokio_tungstenite::connect_async_tls_with_config(request, None, false, connector).await?;
    tracing::info!("tunnel established");
    while let Some(msg) = ws.next().await {
        let msg = msg?;
        // 0.1.1: answer gateway heartbeat pings explicitly (do not rely on
        // library auto-pong); ignore anything that is not a text frame.
        if msg.is_ping() {
            ws.send(Message::Pong(msg.into_data())).await?;
            continue;
        }
        if msg.is_text() {
            let text = msg.to_text()?;
            if let Ok(mut frame) = serde_json::from_str::<serde_json::Value>(text) {
                if let Some(obj) = frame.as_object_mut() {
                    if let Some(headers) = obj.get_mut("headers") {
                        if let Some(map) = headers.as_object_mut() {
                            let mut hm = HeaderMap::new();
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
                    let url = obj.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let method = obj.get("method").and_then(|v| v.as_str()).unwrap_or("GET").to_string();
                    let id = obj.get("id").cloned().unwrap_or(serde_json::Value::String("0".into()));
                    let resp = forward(http_client, &method, &url, &frame).await;
                    let reply = serde_json::json!({
                        "id": id,
                        "status": resp.status,
                        "headers": resp.headers,
                        "body": resp.body,
                    });
                    ws.send(tokio_tungstenite::tungstenite::Message::Text(reply.to_string())).await?;
                }
            }
        }
    }
    Ok(())
}

struct ForwardResp { status: u16, headers: serde_json::Value, body: String }

async fn forward(client: &reqwest::Client, method: &str, url: &str, frame: &serde_json::Value) -> ForwardResp {
    if url.is_empty() {
        return ForwardResp { status: 502, headers: serde_json::json!({}), body: "missing url".into() };
    }
    let mut builder = match method.parse::<reqwest::Method>() {
        Ok(m) => client.request(m, url),
        Err(_) => return ForwardResp { status: 400, headers: serde_json::json!({}), body: "invalid method".into() },
    };
    if let Some(headers) = frame.get("headers").and_then(|v| v.as_object()) {
        for (k, v) in headers {
            if let Some(s) = v.as_str() {
                builder = builder.header(k, s);
            }
        }
    }
    if let Some(body) = frame.get("body").and_then(|v| v.as_str()) {
        if !body.is_empty() {
            builder = builder.body(body.to_string());
        }
    }
    match builder.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let headers = resp.headers().iter().map(|(k, v)| (k.to_string(), serde_json::Value::String(v.to_str().unwrap_or("").to_string()))).collect::<serde_json::Map<_, _>>();
            let body = resp.text().await.unwrap_or_default();
            ForwardResp { status, headers: serde_json::Value::Object(headers), body }
        }
        Err(e) => ForwardResp { status: 502, headers: serde_json::json!({}), body: e.to_string() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_request_has_websocket_headers_and_auth() {
        // R09: the real binary previously sent a bare HTTP request and died
        // with "Missing, duplicated or incorrect header sec-websocket-key".
        let req = build_request("wss://127.0.0.1:18443/ingress", "secret-token").unwrap();
        let headers = req.headers();
        assert!(headers.contains_key("sec-websocket-key"));
        assert_eq!(headers.get("upgrade").unwrap(), "websocket");
        assert_eq!(headers.get("sec-websocket-version").unwrap(), "13");
        assert_eq!(headers.get("authorization").unwrap(), "Bearer secret-token");
    }

    #[test]
    fn backoff_grows_exponentially_and_caps() {
        use std::time::Duration;
        assert_eq!(backoff_delay(1), Duration::from_secs(1));
        assert_eq!(backoff_delay(2), Duration::from_secs(2));
        assert_eq!(backoff_delay(3), Duration::from_secs(4));
        assert_eq!(backoff_delay(6), Duration::from_secs(32));
        assert_eq!(backoff_delay(7), Duration::from_secs(60));
        assert_eq!(backoff_delay(100), Duration::from_secs(60));
        assert_eq!(backoff_delay(0), Duration::from_secs(1));
    }

    #[test]
    fn mask_removes_via_and_forwarded() {
        let mut hm = HeaderMap::new();
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
        let mut hm = HeaderMap::new();
        hm.insert("content-type", "application/json".parse().unwrap());
        mask_headers(&mut hm);
        assert!(hm.contains_key("content-type"));
    }
}

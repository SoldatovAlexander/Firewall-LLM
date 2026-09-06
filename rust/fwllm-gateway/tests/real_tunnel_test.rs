//! R09: real gateway + agent binaries perform a WSS tunnel exchange.
//!
//! Spins up the compiled gateway (TLS ingress) and agent, issues an ingress
//! token, runs a chat completion through the tunnel to a local mock
//! upstream, then verifies wrong-token / wrong-CA rejection and --insecure.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::Request;
use axum::routing::post;

/// Distinct ports per test: tests in one binary run in parallel.
struct Ports {
    http: u16,
    wss: u16,
    upstream: u16,
}
const AGENT_ID: &str = "real-agent";
const ADMIN_KEY: &str = "admin-key-1";
const CLIENT_KEY: &str = "test-client-key";

fn gateway_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fwllm-gateway"))
}

fn agent_bin() -> PathBuf {
    // Same target dir as the gateway binary (workspace build).
    gateway_bin()
        .parent()
        .unwrap()
        .join(format!("fwllm-agent{}", std::env::consts::EXE_SUFFIX))
}

fn ensure_agent_built() {
    if agent_bin().exists() {
        return;
    }
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let status = std::process::Command::new("cargo")
        .args(["build", "-p", "fwllm-agent", "--locked"])
        .current_dir(&workspace)
        .status()
        .expect("cargo build failed to spawn");
    assert!(status.success(), "cargo build -p fwllm-agent failed");
    assert!(agent_bin().exists(), "agent binary missing after build");
}

fn write_config(dir: &std::path::Path, ports: &Ports) {
    let cfg = format!(
        "server:\n  host: 127.0.0.1\n  port: {http}\n\
         providers:\n  tun:\n    type: tunnel\n    base_url: http://127.0.0.1:{up}/v1\n    agent_id: {AGENT_ID}\n\
         routing:\n  default_chain: [tun]\nclients:\n  {CLIENT_KEY}: tester\nadmin_clients:\n  {ADMIN_KEY}: admin\n\
         ingress:\n  enabled: true\n  listen: 127.0.0.1:{wss}\n  sans: [127.0.0.1, localhost]\nquotas: {{}}\naudit:\n  enabled: false\n",
        http = ports.http,
        up = ports.upstream,
        wss = ports.wss,
    );
    std::fs::write(dir.join("fwllm.yaml"), cfg).unwrap();
}

async fn wait_for<F, Fut>(mut probe: F, timeout: Duration, what: &str)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let start = std::time::Instant::now();
    loop {
        if probe().await {
            return;
        }
        if start.elapsed() > timeout {
            panic!("timed out waiting for {what}");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

struct Fixture {
    dir: tempfile::TempDir,
    /// Owns the gateway child process (kill_on_drop); never read directly.
    #[allow(dead_code)]
    gw: tokio::process::Child,
    http: reqwest::Client,
    ports: Ports,
}

impl Fixture {
    async fn start(ports: Ports) -> Self {
        ensure_agent_built();
        let dir = tempfile::tempdir().unwrap();
        write_config(dir.path(), &ports);

        // Local mock upstream: echoes received headers for masking asserts.
        let seen: Arc<Mutex<Option<HashMap<String, String>>>> =
            Arc::new(Mutex::new(None));
        let seen_clone = seen.clone();
        let mock = axum::Router::new().route(
            "/v1/chat/completions",
            post(move |req: Request| {
                let seen_clone = seen_clone.clone();
                async move {
                    let headers: HashMap<String, String> = req
                        .headers()
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                        .collect();
                    *seen_clone.lock().unwrap() = Some(headers);
                    axum::Json(serde_json::json!({
                        "id": "mock-1",
                        "object": "chat.completion",
                        "choices": [{"message": {"role": "assistant", "content": "tunneled-ok"}}],
                        "usage": {"prompt_tokens": 2, "completion_tokens": 2, "total_tokens": 4},
                    }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", ports.upstream))
            .await
            .unwrap();
        tokio::spawn(async move { axum::serve(listener, mock).await.unwrap(); });

        let mut gw = tokio::process::Command::new(gateway_bin())
            .env("FWLLM_CONFIG", dir.path().join("fwllm.yaml"))
            .env("FWLLM_CERTS_DIR", dir.path().join("certs"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("gateway spawn");
        // Fail fast if the gateway exits during startup (bad config/ports).
        let http = reqwest::Client::new();
        let health = format!("http://127.0.0.1:{}/healthz", ports.http);
        let mut started = false;
        for _ in 0..75 {
            if let Ok(r) = http.get(&health).send().await {
                if r.status().is_success() {
                    started = true;
                    break;
                }
            }
            if gw.try_wait().unwrap().is_some() {
                panic!("gateway exited during startup");
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert!(started, "gateway did not become ready");
        Self { dir, gw, http, ports }
    }

    fn certs(&self) -> PathBuf {
        self.dir.path().join("certs")
    }

    fn wss_url(&self) -> String {
        format!("wss://127.0.0.1:{}/ingress", self.ports.wss)
    }

    fn chat_url(&self) -> String {
        format!(
            "http://127.0.0.1:{}/v1/chat/completions",
            self.ports.http
        )
    }

    async fn issue_token(&self, agent: &str) -> String {
        let res = self
            .http
            .post(format!(
                "http://127.0.0.1:{}/admin/ingress/tokens",
                self.ports.http
            ))
            .header("authorization", format!("Bearer {ADMIN_KEY}"))
            .json(&serde_json::json!({"agent_id": agent}))
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), 200);
        res.json::<serde_json::Value>().await.unwrap()["token"]
            .as_str()
            .unwrap()
            .to_string()
    }

    async fn agent_registered(&self) -> bool {
        let res = self
            .http
            .get(format!(
                "http://127.0.0.1:{}/admin/ingress/agents",
                self.ports.http
            ))
            .header("authorization", format!("Bearer {ADMIN_KEY}"))
            .send()
            .await
            .unwrap();
        let body: serde_json::Value = res.json().await.unwrap();
        body["agents"]
            .as_array()
            .map(|agents| {
                agents.iter().any(|a| a["agent_id"].as_str() == Some(AGENT_ID))
            })
            .unwrap_or(false)
    }

    fn spawn_agent(&self, extra_args: &[&str]) -> tokio::process::Child {
        // Inherit stdio when debugging (FWLLM_TEST_VERBOSE=1); passing
        // tests stay quiet.
        let mut cmd = tokio::process::Command::new(agent_bin());
        cmd.args(extra_args).kill_on_drop(true);
        if std::env::var("FWLLM_TEST_VERBOSE").is_err() {
            cmd.stdout(Stdio::null()).stderr(Stdio::null());
        }
        cmd.spawn().expect("agent spawn")
    }
}

#[tokio::test]
async fn real_agent_tunnel_completion() {
    let fix = Fixture::start(Ports { http: 18081, wss: 18443, upstream: 18082 }).await;
    let token = fix.issue_token(AGENT_ID).await;
    let ca = fix.certs().join("ca.crt");
    assert!(ca.exists(), "gateway must generate ca.crt");
    let wss = fix.wss_url();
    let ca_str = ca.to_str().unwrap().to_string();
    let mut agent = fix.spawn_agent(&[
        "--gateway-url",
        &wss,
        "--token",
        &token,
        "--ca-cert",
        &ca_str,
    ]);
    wait_for(
        || async { fix.agent_registered().await },
        Duration::from_secs(15),
        "agent tunnel registration",
    )
    .await;

    let res = fix
        .http
        .post(fix.chat_url())
        .header("authorization", format!("Bearer {CLIENT_KEY}"))
        .json(&serde_json::json!({
            "model": "any",
            "messages": [{"role": "user", "content": "hi"}],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 200);
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "tunneled-ok");
    agent.kill().await.unwrap();
}

#[tokio::test]
async fn real_agent_wrong_token_rejected() {
    let fix = Fixture::start(Ports { http: 18181, wss: 18543, upstream: 18182 }).await;
    let wss = fix.wss_url();
    let mut agent = fix.spawn_agent(&[
        "--gateway-url",
        &wss,
        "--token",
        "wrong-token-xyz",
        "--insecure",
    ]);
    let status = tokio::time::timeout(Duration::from_secs(20), agent.wait())
        .await
        .expect("agent with wrong token should exit")
        .unwrap();
    assert!(!status.success(), "wrong token must fail the handshake");
}

#[tokio::test]
async fn real_agent_wrong_ca_rejected() {
    let fix = Fixture::start(Ports { http: 18281, wss: 18643, upstream: 18282 }).await;
    let token = fix.issue_token(AGENT_ID).await;
    // A different, valid self-signed CA: parses fine but does not verify.
    let other = rcgen::generate_simple_self_signed(vec!["other".to_string()]).unwrap();
    let other_ca = fix.dir.path().join("other-ca.crt");
    std::fs::write(&other_ca, other.cert.pem()).unwrap();
    let wss = fix.wss_url();
    let other_str = other_ca.to_str().unwrap().to_string();
    let mut agent = fix.spawn_agent(&[
        "--gateway-url",
        &wss,
        "--token",
        &token,
        "--ca-cert",
        &other_str,
    ]);
    let status = tokio::time::timeout(Duration::from_secs(20), agent.wait())
        .await
        .expect("agent with wrong CA should exit")
        .unwrap();
    assert!(!status.success(), "wrong CA must fail TLS verification");
}

#[tokio::test]
async fn real_agent_insecure_connects() {
    let fix = Fixture::start(Ports { http: 18381, wss: 18743, upstream: 18382 }).await;
    let token = fix.issue_token(AGENT_ID).await;
    let wss = fix.wss_url();
    let mut agent = fix.spawn_agent(&[
        "--gateway-url",
        &wss,
        "--token",
        &token,
        "--insecure",
    ]);
    wait_for(
        || async { fix.agent_registered().await },
        Duration::from_secs(15),
        "insecure agent registration",
    )
    .await;
    agent.kill().await.unwrap();
}

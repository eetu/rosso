//! Spawns the real `rosso-backend` binary against a temp SQLite and a stub
//! `dist/`, polls `/status` until it answers, and kills the child on `Drop`.
//!
//! The backend is real here; the frontend is absent and third-party HTTP (feeds,
//! Ollama) is wiremocked. That makes these *integration* tests, not e2e — the
//! name e2e is reserved for a real-SPA-against-real-backend suite.
//!
//! Tests are `#[ignore]` because they bind a port and spawn a process:
//! `cargo test -p rosso-integration -- --ignored`.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Duration;

use tempfile::TempDir;

pub struct Stack {
    child: Child,
    pub base: String,
    pub client: reqwest::Client,
    _data_tmp: TempDir,
    _static_tmp: TempDir,
}

impl Stack {
    pub async fn start() -> anyhow::Result<Self> {
        Self::start_with_env(&[]).await
    }

    /// Like [`Stack::start`] but with extra env vars — e.g. pointing
    /// `ROSSO_OLLAMA_URL` at a wiremock stub, or at a dead port to prove the
    /// reader still works without the LLM.
    pub async fn start_with_env(extra: &[(&str, &str)]) -> anyhow::Result<Self> {
        let data_tmp = tempfile::tempdir()?;
        let db_path = data_tmp.path().join("rosso.db");

        // A stub dist/ so serve_spa's index.html fallback resolves.
        let static_tmp = tempfile::tempdir()?;
        // Carries an inline script on purpose: the CSP hashes what the shell
        // actually contains, so a shell without one would prove nothing.
        std::fs::write(
            static_tmp.path().join("index.html"),
            "<html><body>rosso<script>window.__rosso = 1;</script></body></html>",
        )?;

        let port = free_port()?;
        let base = format!("http://127.0.0.1:{port}");

        // Build the client *before* spawning: `reqwest` panics rather than
        // erroring when the rustls provider is missing, and a panic between
        // spawn and the `Stack` that owns the child leaks the backend process —
        // it outlives the test run with no Drop to kill it.
        install_crypto_provider();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()?;

        let mut cmd = Command::new(bin_path());
        cmd.env("DEV_AUTH", "1") // bypass forward-auth so /api/* is reachable
            .env("ROSSO_BIND", format!("127.0.0.1:{port}"))
            .env("ROSSO_DB_PATH", &db_path)
            .env("STATIC_DIR", static_tmp.path())
            .env("ROSSO_SHUTDOWN_GRACE_S", "0")
            // wiremock serves from 127.0.0.1, which extraction refuses by
            // default as an SSRF guard. Without this the extraction tests would
            // "pass" by being refused before they reach what they test.
            .env("ROSSO_EXTRACT_ALLOW_PRIVATE", "1")
            .env("RUST_LOG", "warn");
        for (k, v) in extra {
            cmd.env(k, v);
        }
        let child = cmd.spawn()?;

        // Generous: the suite spawns several backends in parallel, so startup can
        // lag behind the first probe.
        let mut up = false;
        for _ in 0..200 {
            if let Ok(r) = client.get(format!("{base}/status")).send().await {
                if r.status().is_success() {
                    up = true;
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let stack = Stack {
            child,
            base,
            client,
            _data_tmp: data_tmp,
            _static_tmp: static_tmp,
        };
        if !up {
            anyhow::bail!("backend did not come up within 20s");
        }
        Ok(stack)
    }

    pub async fn get(&self, route: &str) -> reqwest::Response {
        self.client
            .get(format!("{}{route}", self.base))
            .send()
            .await
            .expect("request failed")
    }

    pub async fn get_json(&self, route: &str) -> serde_json::Value {
        let r = self.get(route).await;
        assert!(r.status().is_success(), "GET {route} → {}", r.status());
        r.json().await.expect("json")
    }

    pub async fn post(&self, route: &str, body: serde_json::Value) -> reqwest::Response {
        self.client
            .post(format!("{}{route}", self.base))
            .json(&body)
            .send()
            .await
            .expect("request failed")
    }

    pub async fn post_json(&self, route: &str, body: serde_json::Value) -> serde_json::Value {
        let r = self.post(route, body).await;
        assert!(r.status().is_success(), "POST {route} → {}", r.status());
        r.json().await.expect("json")
    }

    /// A body that is not JSON — the OPML import takes a file's text.
    pub async fn post_text(&self, route: &str, body: &str) -> reqwest::Response {
        self.client
            .post(format!("{}{route}", self.base))
            .header("content-type", "text/xml")
            .body(body.to_string())
            .send()
            .await
            .expect("request failed")
    }

    pub async fn put_json(&self, route: &str, body: serde_json::Value) -> serde_json::Value {
        let r = self
            .client
            .put(format!("{}{route}", self.base))
            .json(&body)
            .send()
            .await
            .expect("request failed");
        assert!(r.status().is_success(), "PUT {route} → {}", r.status());
        r.json().await.expect("json")
    }

    pub async fn patch_json(&self, route: &str, body: serde_json::Value) -> serde_json::Value {
        let r = self
            .client
            .patch(format!("{}{route}", self.base))
            .json(&body)
            .send()
            .await
            .expect("request failed");
        assert!(r.status().is_success(), "PATCH {route} → {}", r.status());
        r.json().await.expect("json")
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The workspace builds reqwest with `rustls-no-provider`, so every crate that
/// makes a client installs one. Tests share a process, hence the `Once`.
fn install_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

pub fn free_port() -> anyhow::Result<u16> {
    let l = TcpListener::bind("127.0.0.1:0")?;
    Ok(l.local_addr()?.port())
}

/// Resolve the sibling backend binary next to the test runner (works under
/// `cargo test`, which builds into `target/<profile>/deps/`).
///
/// Cargo does not rebuild another package's binary for `cargo test -p
/// rosso-integration`, so the binary found here is only as fresh as the last
/// build. Run the suite through `just test-integration`, which builds first —
/// otherwise a passing run proves nothing about the current source.
fn bin_path() -> PathBuf {
    let mut p = std::env::current_exe().expect("current_exe");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    let bin = p.join("rosso-backend");
    assert!(
        bin.is_file(),
        "{} not built — run `just test-integration`",
        bin.display()
    );
    bin
}

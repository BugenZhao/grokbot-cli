//! Minimal Connect-protocol client (unary, JSON codec) for Cursor's `aiserver.v1` backend.
//!
//! Every RPC is `POST {base}/aiserver.v1.{Service}/{Method}` with a protobuf-JSON body.
//! See docs/protocol.md for the headers and why JSON (not binary protobuf) is used.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use base64::Engine;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;

pub const DEFAULT_BACKEND: &str = "https://api2.cursor.sh";
/// Version reported in client headers. The backend gates features on it, so keep it
/// close to the desktop app version the schema was extracted from (schema/services.txt).
pub const CLIENT_VERSION: &str = "0.75.0";

/// A Connect error returned by the server (`{"code": "...", "message": "..."}`).
#[derive(Debug)]
pub struct RpcError {
    pub method: String,
    pub http_status: u16,
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} failed: {} ({})",
            self.method, self.code, self.http_status
        )?;
        if !self.message.is_empty() {
            write!(f, ": {}", self.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for RpcError {}

impl RpcError {
    pub fn is_unauthenticated(&self) -> bool {
        self.code == "unauthenticated" || self.http_status == 401
    }
}

pub struct Client {
    http: reqwest::blocking::Client,
    base: String,
    token: String,
    machine_id: String,
    pub verbose: bool,
}

impl Client {
    pub fn new(base: &str, token: String, machine_id: String) -> Result<Self> {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            token,
            machine_id,
            verbose: false,
        })
    }

    /// Call `method` (`"GrokBotService/ListGrokBotAgents"`, package prefix optional).
    pub fn call<Q: Serialize, R: DeserializeOwned>(&self, method: &str, req: &Q) -> Result<R> {
        let value = self.call_value(method, &serde_json::to_value(req)?)?;
        serde_json::from_value(value).with_context(|| format!("decoding {method} response"))
    }

    pub fn call_value(&self, method: &str, body: &Value) -> Result<Value> {
        let path = if method.starts_with("aiserver.") {
            method.to_owned()
        } else {
            format!("aiserver.v1.{method}")
        };
        let url = format!("{}/{path}", self.base);
        if self.verbose {
            eprintln!("> POST {url}\n> {body}");
        }
        let resp = self
            .http
            .post(&url)
            .headers(client_headers(&self.token, &self.machine_id))
            .json(body)
            .send()
            .with_context(|| format!("sending {method}"))?;
        let status = resp.status();
        let text = resp.text()?;
        if self.verbose {
            eprintln!("< {status}\n< {text}");
        }
        if !status.is_success() {
            let err: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            return Err(RpcError {
                method: method.to_owned(),
                http_status: status.as_u16(),
                code: err["code"].as_str().unwrap_or("unknown").to_owned(),
                message: err["message"].as_str().unwrap_or(&text).to_owned(),
            }
            .into());
        }
        if text.is_empty() {
            return Ok(Value::Object(Default::default()));
        }
        serde_json::from_str(&text).with_context(|| format!("parsing {method} response"))
    }

    /// Plain PUT of bytes to a presigned URL (attachment upload).
    pub fn put_presigned(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: Vec<u8>,
    ) -> Result<()> {
        let mut req = self
            .http
            .put(url)
            .body(body)
            .timeout(Duration::from_secs(600));
        for (k, v) in headers {
            req = req.header(k, v);
        }
        let resp = req.send()?;
        anyhow::ensure!(
            resp.status().is_success(),
            "upload failed: HTTP {}",
            resp.status()
        );
        Ok(())
    }
}

/// Headers the desktop app sends on every backend RPC (`VTe` + `getSandBackendClientHeaders`
/// in the app bundle; see docs/protocol.md).
fn client_headers(token: &str, machine_id: &str) -> reqwest::header::HeaderMap {
    let mut h = reqwest::header::HeaderMap::new();
    let mut set = |k: &'static str, v: String| {
        h.insert(k, v.parse().expect("header value"));
    };
    set("content-type", "application/json".into());
    set("connect-protocol-version", "1".into());
    set("authorization", format!("Bearer {token}"));
    set(
        "user-agent",
        format!(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Sand/{CLIENT_VERSION} Safari/537.36"
        ),
    );
    set("x-cursor-client-type", "sand".into());
    set("x-cursor-client-version", CLIENT_VERSION.into());
    // The app sends Node's `process.platform` ("darwin", "linux", "win32").
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    set("x-cursor-client-os", os.into());
    set("x-cursor-client-source", "sand-desktop".into());
    set("x-sand-box-namespace", "prod".into());
    set("x-ghost-mode", "true".into());
    set("x-cursor-checksum", checksum(machine_id));
    set("x-request-id", uuid::Uuid::new_v4().to_string());
    h
}

/// `x-cursor-checksum`: obfuscated coarse timestamp followed by the machine id (`sS`/`pWt`).
fn checksum(machine_id: &str) -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    checksum_at(ms, machine_id)
}

fn checksum_at(ms: u64, machine_id: &str) -> String {
    let t = ms / 1_000_000;
    let mut bytes: Vec<u8> = (0..6).rev().map(|i| (t >> (i * 8)) as u8).collect();
    let mut prev = 165u8;
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = (*b ^ prev).wrapping_add((i % 256) as u8);
        prev = *b;
    }
    format!(
        "{}{machine_id}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_matches_app_algorithm() {
        // Expected value computed independently from the app's JS (`sS`/`pWt`).
        assert_eq!(checksum_at(1_792_637_601_155, "abc"), "paaotvCSabc");
    }

    #[test]
    fn rpc_error_auth_detection() {
        let e = |code: &str, status| RpcError {
            method: "M".into(),
            http_status: status,
            code: code.into(),
            message: String::new(),
        };
        assert!(e("unauthenticated", 401).is_unauthenticated());
        assert!(e("unknown", 401).is_unauthenticated());
        assert!(!e("not_found", 404).is_unauthenticated());
        assert_eq!(e("not_found", 404).to_string(), "M failed: not_found (404)");
    }
}

//! Cursor's PKCE "deep control" login, the same flow the desktop app uses.
//!
//! 1. `https://cursor.com/loginDeepControl?challenge=…&uuid=…&mode=login&redirectTarget=cli`
//! 2. poll `GET {backend}/auth/poll?uuid=…&verifier=…` until it returns the token pair
//! 3. later, `POST {backend}/oauth/token` with `grant_type=refresh_token`

use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use base64::Engine;
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::NotLoggedIn;

const WEBSITE: &str = "https://cursor.com";
/// OAuth client id the app uses for production backends (`HW` in the app bundle).
const AUTH_CLIENT_ID: &str = "KbZUR41cY7W6zRSdpSUJ7I7mLYBKOCmB";

pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: Option<String>,
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Run the interactive browser login and return (access, refresh).
pub fn login(backend: &str, open_browser: bool, timeout: Duration) -> Result<(String, String)> {
    let mut raw = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw);
    let verifier = b64url(&raw);
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    let uuid = uuid::Uuid::new_v4().to_string();
    let url = format!(
        "{WEBSITE}/loginDeepControl?challenge={challenge}&uuid={uuid}&mode=login&redirectTarget=cli"
    );

    eprintln!("Open this URL to sign in:\n\n  {url}\n");
    if open_browser && open::that(&url).is_ok() {
        eprintln!("(opened in your browser; waiting for sign-in…)");
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Poll {
        access_token: Option<String>,
        refresh_token: Option<String>,
    }

    let http = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    let deadline = Instant::now() + timeout;
    let mut errors = 0;
    while Instant::now() < deadline {
        let resp = http
            .get(format!("{backend}/auth/poll"))
            .query(&[("uuid", &uuid), ("verifier", &verifier)])
            .send();
        match resp {
            // 404 means "not signed in yet".
            Ok(r) if r.status() == 404 => errors = 0,
            Ok(r) if r.status().is_success() => {
                let p: Poll = r.json().context("parsing /auth/poll response")?;
                if let (Some(a), Some(r)) = (p.access_token, p.refresh_token) {
                    return Ok((a, r));
                }
                bail!("/auth/poll returned 200 without a token pair");
            }
            Ok(r) if r.status() == 403 => bail!("sign-in was refused by the server (HTTP 403)"),
            _ => {
                errors += 1;
                if errors >= 5 {
                    bail!("repeated errors while polling /auth/poll");
                }
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    bail!("timed out waiting for sign-in")
}

pub fn refresh(backend: &str, refresh_token: &str) -> Result<TokenPair> {
    #[derive(Deserialize)]
    struct Resp {
        access_token: Option<String>,
        refresh_token: Option<String>,
        #[serde(rename = "shouldLogout", default)]
        should_logout: bool,
    }
    let resp = reqwest::blocking::Client::new()
        .post(format!("{backend}/oauth/token"))
        .json(&serde_json::json!({
            "client_id": AUTH_CLIENT_ID,
            "grant_type": "refresh_token",
            "refresh_token": refresh_token,
        }))
        .timeout(Duration::from_secs(30))
        .send()
        .context("refreshing the access token")?;
    let status = resp.status();
    let body: Resp = resp.json().unwrap_or(Resp {
        access_token: None,
        refresh_token: None,
        should_logout: false,
    });
    match body.access_token {
        Some(a) if status.is_success() && !body.should_logout => Ok(TokenPair {
            access_token: a,
            refresh_token: body.refresh_token,
        }),
        _ => Err(NotLoggedIn(format!(
            "session expired (token refresh failed, HTTP {status}); run `grokbot login` again"
        ))
        .into()),
    }
}

/// A stable per-install machine id (64 hex chars, like the app's).
pub fn new_machine_id() -> String {
    let mut raw = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw);
    hex::encode(Sha256::digest(raw))
}

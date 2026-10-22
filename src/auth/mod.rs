//! Credentials: either our own Cursor session (`grokbot login`) or a read-only view of the
//! desktop app's session (`grokbot login --from-app`). See docs/auth.md.

pub mod app;
pub mod login;

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::connect::{Client, DEFAULT_BACKEND, RpcError};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Stored {
    /// Our own session from the PKCE login; we own (and refresh) these tokens.
    Login {
        access_token: String,
        refresh_token: String,
        machine_id: String,
    },
    /// Borrow the desktop app's session. Only a cached copy of the access token is kept
    /// so the keychain is not hit on every invocation; it is never refreshed here.
    App {
        #[serde(default)]
        cached_access_token: Option<String>,
        #[serde(default)]
        machine_id: Option<String>,
    },
}

pub fn config_dir(profile: &str) -> Result<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(p) => PathBuf::from(p),
        None => dirs::home_dir()
            .context("no home directory")?
            .join(".config"),
    };
    let dir = base.join("grokbot");
    Ok(if profile == "default" {
        dir
    } else {
        dir.join("profiles").join(profile)
    })
}

fn auth_path(profile: &str) -> Result<PathBuf> {
    Ok(config_dir(profile)?.join("auth.json"))
}

pub fn load(profile: &str) -> Result<Option<Stored>> {
    let path = auth_path(profile)?;
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(Some(
            serde_json::from_str(&s).with_context(|| format!("parsing {}", path.display()))?,
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn save(profile: &str, stored: &Stored) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let path = auth_path(profile)?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&path)?;
    f.write_all(serde_json::to_string_pretty(stored)?.as_bytes())?;
    Ok(())
}

pub fn remove(profile: &str) -> Result<bool> {
    match std::fs::remove_file(auth_path(profile)?) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// Error shown when there is no usable session; maps to exit code 3.
#[derive(Debug)]
pub struct NotLoggedIn(pub String);

impl std::fmt::Display for NotLoggedIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NotLoggedIn {}

/// `exp` claim of a JWT, if it parses.
fn jwt_exp(token: &str) -> Option<u64> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    serde_json::from_slice::<serde_json::Value>(&bytes).ok()?["exp"].as_u64()
}

fn expires_soon(token: &str) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    jwt_exp(token).is_some_and(|exp| exp <= now + 60)
}

/// An authenticated client plus enough state to recover from one 401.
pub struct Session {
    pub client: Client,
    profile: String,
    stored: Stored,
}

impl Session {
    pub fn open(profile: &str, verbose: bool) -> Result<Self> {
        if let Ok(token) = std::env::var("GROKBOT_TOKEN") {
            let mut client = Client::new(DEFAULT_BACKEND, token, String::new())?;
            client.verbose = verbose;
            let stored = Stored::App {
                cached_access_token: None,
                machine_id: None,
            };
            return Ok(Self {
                client,
                profile: profile.to_owned(),
                stored,
            });
        }
        let mut stored = load(profile)?.ok_or_else(|| {
            NotLoggedIn("not logged in; run `grokbot login` (or `grokbot login --from-app`)".into())
        })?;
        let (token, machine_id) = fresh_token(profile, &mut stored, false)?;
        let mut client = Client::new(DEFAULT_BACKEND, token, machine_id)?;
        client.verbose = verbose;
        Ok(Self {
            client,
            profile: profile.to_owned(),
            stored,
        })
    }

    /// Run `f`; on an authentication failure renew the token once and retry.
    pub fn with_retry<T>(&mut self, mut f: impl FnMut(&Client) -> Result<T>) -> Result<T> {
        match f(&self.client) {
            Err(e)
                if e.downcast_ref::<RpcError>()
                    .is_some_and(RpcError::is_unauthenticated) =>
            {
                if std::env::var("GROKBOT_TOKEN").is_ok() {
                    return Err(e);
                }
                let (token, machine_id) = fresh_token(&self.profile, &mut self.stored, true)?;
                let verbose = self.client.verbose;
                self.client = Client::new(DEFAULT_BACKEND, token, machine_id)?;
                self.client.verbose = verbose;
                f(&self.client)
            }
            r => r,
        }
    }
}

/// Return a usable access token, renewing it when expired (or when `force`).
fn fresh_token(profile: &str, stored: &mut Stored, force: bool) -> Result<(String, String)> {
    match stored {
        Stored::Login {
            access_token,
            refresh_token,
            machine_id,
        } => {
            if force || expires_soon(access_token) {
                let pair = login::refresh(DEFAULT_BACKEND, refresh_token)?;
                *access_token = pair.access_token;
                if let Some(r) = pair.refresh_token {
                    *refresh_token = r;
                }
                save(profile, stored)?;
                if let Stored::Login {
                    access_token,
                    machine_id,
                    ..
                } = stored
                {
                    return Ok((access_token.clone(), machine_id.clone()));
                }
                unreachable!()
            }
            Ok((access_token.clone(), machine_id.clone()))
        }
        Stored::App {
            cached_access_token,
            machine_id,
        } => {
            if let (false, Some(t), Some(m)) = (force, &cached_access_token, &machine_id)
                && !expires_soon(t)
            {
                return Ok((t.clone(), m.clone()));
            }
            let creds = app::read()?;
            if force && cached_access_token.as_deref() == Some(creds.access_token.as_str()) {
                return Err(NotLoggedIn(
                    "the Grok Bot app's session was rejected; open the app so it can refresh its sign-in, then retry"
                        .into(),
                )
                .into());
            }
            *cached_access_token = Some(creds.access_token.clone());
            *machine_id = Some(creds.machine_id.clone());
            save(profile, stored)?;
            Ok((creds.access_token, creds.machine_id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt(claims: &str) -> String {
        let b = |s: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s);
        format!("{}.{}.sig", b(r#"{"alg":"none"}"#), b(claims))
    }

    #[test]
    fn jwt_expiry() {
        assert_eq!(jwt_exp(&jwt(r#"{"exp":1234}"#)), Some(1234));
        assert_eq!(jwt_exp("not-a-jwt"), None);
        assert!(expires_soon(&jwt(r#"{"exp":1}"#)));
        assert!(!expires_soon(&jwt(r#"{"exp":99999999999}"#)));
        // Unparseable tokens are treated as valid; the server decides.
        assert!(!expires_soon("opaque"));
    }

    #[test]
    fn stored_round_trip() {
        let s = Stored::Login {
            access_token: "a".into(),
            refresh_token: "r".into(),
            machine_id: "m".into(),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""mode":"login""#));
        assert!(matches!(
            serde_json::from_str(&json).unwrap(),
            Stored::Login { .. }
        ));
        let app: Stored = serde_json::from_str(r#"{"mode":"app"}"#).unwrap();
        assert!(matches!(
            app,
            Stored::App {
                cached_access_token: None,
                machine_id: None
            }
        ));
    }
}

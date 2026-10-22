//! Read-only import of the Grok Bot desktop app's sign-in (`grokbot login --from-app`).
//!
//! The app keeps tokens in `~/Library/Application Support/Grok Bot/sand-secrets.json`,
//! encrypted with Electron `safeStorage` (Chromium OSCrypt "v10" on macOS). We never write
//! that file and never refresh its tokens: the app may rotate the refresh token, and using
//! it here would sign the app out. See docs/auth.md.

use std::path::PathBuf;

use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use anyhow::{Context, Result, bail};
use base64::Engine;
use serde_json::Value;

const KEYCHAIN_SERVICE: &str = "Grok Bot Safe Storage";
const KEYCHAIN_ACCOUNT: &str = "Grok Bot Key";

pub struct AppCredentials {
    pub access_token: String,
    pub machine_id: String,
}

fn secrets_path() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("no home directory")?
        .join("Library/Application Support/Grok Bot/sand-secrets.json"))
}

pub fn read() -> Result<AppCredentials> {
    let path = secrets_path()?;
    let raw = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "reading {} (is the Grok Bot app installed and signed in?)",
            path.display()
        )
    })?;
    let file: Value = serde_json::from_str(&raw).context("sand-secrets.json is not JSON")?;
    let accounts: Value = serde_json::from_str(
        file["cursor-accounts"]
            .as_str()
            .context("sand-secrets.json has no cursor-accounts (format changed?)")?,
    )?;
    let active = accounts["active"]
        .as_str()
        .context("the Grok Bot app has no active account; sign in there first")?;
    let account = &accounts["accounts"][active];

    let key = derive_key()?;
    let decrypt_field = |v: &Value, what: &str| -> Result<String> {
        let s = v
            .as_str()
            .with_context(|| format!("missing {what} in sand-secrets.json"))?;
        decrypt(&key, s).with_context(|| format!("decrypting {what}"))
    };
    let access_token = decrypt_field(&account["cursor-access-token"], "cursor-access-token")?;
    let machine_id = decrypt_field(&file["cursor-machine-id"], "cursor-machine-id")?;
    Ok(AppCredentials {
        access_token,
        machine_id,
    })
}

/// OSCrypt key: PBKDF2-HMAC-SHA1(keychain password, "saltysalt", 1003 rounds, 16 bytes).
fn derive_key() -> Result<[u8; 16]> {
    let password = security_framework::passwords::get_generic_password(
        KEYCHAIN_SERVICE,
        KEYCHAIN_ACCOUNT,
    )
    .context(
        "reading the \"Grok Bot Safe Storage\" keychain item (denied, or app never signed in)",
    )?;
    let mut key = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(&password, b"saltysalt", 1003, &mut key);
    Ok(key)
}

/// Values are base64("v10" || AES-128-CBC(PKCS7, iv = 16 spaces)). An optional
/// `plaintext:v1:` prefix means the app stored it unencrypted (no keychain available).
fn decrypt(key: &[u8; 16], value: &str) -> Result<String> {
    if let Some(plain) = value.strip_prefix("plaintext:v1:") {
        return Ok(plain.to_owned());
    }
    let value = value.strip_prefix("scoped:v1:").unwrap_or(value);
    let bytes = base64::engine::general_purpose::STANDARD.decode(value)?;
    let Some(ct) = bytes.strip_prefix(b"v10") else {
        bail!("unexpected ciphertext version (expected v10)");
    };
    let iv = [b' '; 16];
    let pt = cbc::Decryptor::<aes::Aes128>::new(key.into(), &iv.into())
        .decrypt_padded_vec_mut::<Pkcs7>(ct)
        .map_err(|_| anyhow::anyhow!("bad padding (wrong keychain key?)"))?;
    Ok(String::from_utf8(pt)?)
}

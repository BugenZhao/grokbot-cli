# Authentication

Two modes, stored in `~/.config/grokbot[/profiles/NAME]/auth.json` (mode 0600).
`$GROKBOT_TOKEN` overrides both (raw access token, no renewal) — handy for CI.

## `grokbot login` — own session (default, recommended)

Cursor's PKCE “deep control” login, same as the app (`startLogin` in `main-app.cjs`):

1. `verifier = base64url(32 random bytes)`, `challenge = base64url(sha256(verifier))`, `uuid` random.
2. Open `https://cursor.com/loginDeepControl?challenge=…&uuid=…&mode=login&redirectTarget=cli`.
3. Poll `GET https://api2.cursor.sh/auth/poll?uuid=…&verifier=…` once a second:
   `404` = not yet, `200` = `{accessToken, refreshToken, authId}`, `403` = refused.
4. Renew with `POST https://api2.cursor.sh/oauth/token`
   `{"client_id":"KbZUR41cY7W6zRSdpSUJ7I7mLYBKOCmB","grant_type":"refresh_token","refresh_token":…}`
   → `{access_token, refresh_token?, shouldLogout?}`. Done when the JWT `exp` is within 60 s
   or after a 401.

This session is independent of the app, so renewing never signs the app out.

> Status: the URL/poll steps are verified; a full browser sign-in with
> `redirectTarget=cli` has not been exercised yet.

## `grokbot login --from-app` — borrow the desktop app's session

Read-only. Source: `~/Library/Application Support/Grok Bot/sand-secrets.json`

```jsonc
{
  "version": 1,
  "cursor-machine-id": "<enc>",
  "cursor-accounts": "{\"active\":\"<64-hex scope>\",\"accounts\":{\"<scope>\":{
      \"cursor-access-token\":\"<enc>\",\"cursor-refresh-token\":\"<enc>\",\"cursor-account-profile\":\"<enc>\"}}}",
  "local-exec-file-key": "<enc>"
}
```

`<enc>` is Electron `safeStorage` = Chromium OSCrypt on macOS:

- password: keychain generic password, service `Grok Bot Safe Storage`, account `Grok Bot Key`
  (macOS asks for permission on first access, and again after the binary changes);
- key: `PBKDF2-HMAC-SHA1(password, "saltysalt", 1003 iterations, 16 bytes)`;
- value: `base64("v10" || AES-128-CBC(key, iv = 16 × 0x20, PKCS#7))`.
  A `plaintext:v1:` prefix means stored unencrypted; `scoped:v1:` is stripped.

Rules this mode follows:

- **Never refresh** the app's token: the server may rotate refresh tokens, and using it
  here would sign the app out. Only the access token is used.
- A copy of the access token is cached in `auth.json` until its JWT `exp`, so the keychain
  is not read on every run. On expiry/401 the app's file is re-read; if it still holds
  the same (rejected) token, the CLI exits 3 asking you to open the app so it refreshes.
- The secrets file layout is an app implementation detail; `src/auth/app.rs` fails with a
  descriptive error rather than guessing when it changes.

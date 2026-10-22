# Wire protocol

The backend speaks [Connect](https://connectrpc.com/docs/protocol) (gRPC-compatible, over
plain HTTP). The app uses `@connectrpc/connect-node` with HTTP/1.1 and binary protobuf; the
server also accepts the **JSON codec**, which is what this CLI uses — no codegen, and
unknown fields are simply ignored.

## Unary call

```
POST https://api2.cursor.sh/aiserver.v1.<Service>/<Method>
Content-Type: application/json
Connect-Protocol-Version: 1
<client headers, below>

{ ...request in protobuf-JSON... }
```

- Success: HTTP 200, body is the response message (`{}` when all fields are default).
- Error: non-2xx with `{"code":"not_found","message":"…","details":[…]}`. `details[].debug`
  often carries a readable reason, e.g. `"Grok Bot routines are not available for this bot."`
  (`grokbot -v` shows it). `401`/`unauthenticated` triggers one token renewal and retry.

protobuf-JSON conventions: camelCase field names; int64/uint64 as **strings**; enums as
full names (`GROK_BOT_SEND_STATUS_ACCEPTED`); `bytes` as base64; default values omitted.

Server-streaming RPCs (e.g. `WatchGrokBotTranscripts`) use Connect's enveloped
`application/connect+json` framing. The CLI does not use any; it polls instead.

## Client headers

Copied from the app (`getSandBackendClientHeaders` and `VTe` in `main-app.cjs`):

| Header | Value | Notes |
|---|---|---|
| `authorization` | `Bearer <access token>` | |
| `user-agent` | `Mozilla/5.0 (Macintosh; …) AppleWebKit/537.36 (KHTML, like Gecko) Sand/<ver> Safari/537.36` | |
| `x-cursor-client-type` | `sand` | Grok Bot's internal codename is “Sand” |
| `x-cursor-client-version` | `0.75.0` | `CLIENT_VERSION` in `src/connect.rs`; keep near the app |
| `x-cursor-client-os` | `darwin`/`linux`/`win32` | Node's `process.platform` |
| `x-cursor-client-source` | `sand-desktop` | |
| `x-sand-box-namespace` | `prod` | |
| `x-ghost-mode` | `true` | privacy mode; app derives it from `GetUserPrivacyMode` |
| `x-cursor-checksum` | `base64url(obf(ts)) + machineId` | see below |
| `x-request-id` | random UUID | |

`x-cursor-checksum` (`sS`/`pWt`): `t = floor(now_ms / 1e6)` as 6 big-endian bytes, then
`b[i] = (b[i] ^ prev) + i` with `prev` starting at 165 and becoming the new `b[i]`;
base64url without padding, followed by the 64-hex machine id. In `--from-app` mode the
app's machine id is used; `grokbot login` generates one per profile.

The app sends `x-cursor-timezone` on `SendGrokBotUserMessage`; the CLI passes the time
zone in request bodies where the schema has a `time_zone` field instead.

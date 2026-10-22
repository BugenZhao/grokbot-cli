# Schema: where it comes from and how to keep up

The backend is Cursor's `aiserver.v1` protobuf API. There is no public `.proto`, so we
reconstruct it from the Grok Bot desktop app, which ships the protobuf-es generated code.

## Files

| File | What | Edited by |
|---|---|---|
| `schema/aiserver.proto` | Every service, message and enum in the app (≈550 RPCs, 2.5k messages) | `tools/extract_schema.py` |
| `schema/services.txt` | One line per RPC: `Service/Method  Req -> Res  [streaming kind]` | `tools/extract_schema.py` |
| `schema/used.proto` | Only the RPCs `src/` calls, plus every type they reach | `tools/check_rpcs.py --write` |
| `src/api.rs` | Hand-written serde structs for the fields we actually read/write | you |
| `src/transcript.rs` | Decoder for transcript entry bodies (JSON, **not** in the schema) | you |

Generated files are committed so that `git diff` after an app update is the review.

## Updating after a Grok Bot release

```sh
just update-schema                # installed app if present, else download the latest build
just update-schema --download     # always download (any OS; no Grok Bot install needed)
just update-schema "/path/Grok Bot.app"
```

`tools/update-schema.sh` runs: fetch (if needed) → extract → `protoc` check (skipped if
not installed) → used-RPC check → diff summary. Needs only `python3` and `git`.

Then:

1. **Missing RPC** (`check_rpcs.py` prints `MISSING`, exit 1): the method was renamed or
   removed. Search `schema/services.txt` for a successor and look at how the app now does
   the same thing (see “Reading the app's behaviour” below).
2. **Diff in `schema/used.proto`**: for each changed message, check whether `src/api.rs`
   uses the field. Additive changes (new optional fields, new enum values) are harmless:
   serde ignores unknown fields and enums are kept as strings. Renamed/renumbered fields
   or changed types need an `api.rs` edit.
3. Bump `CLIENT_VERSION` in `src/connect.rs` to the app version (the backend may gate
   behaviour on `x-cursor-client-version`).
4. Smoke-test against a scratch bot: `grokbot ask`, `log`, `status`, `routines`.
5. Optionally diff `services.txt` for new RPCs worth exposing.

Transcript bodies are JSON blobs with no schema; if `log`/`ask` print nothing or
`[needs input]` stops appearing, dump raw entries (`grokbot log --json -n 50`) and update
`src/transcript.rs` + `docs/transcript.md`.

## Getting the app without installing it

`tools/fetch_app.py` (used by `--download`) asks the app's own update feed for the
newest build, the same request the app makes (`buildUpdateRequestUrl` in `main-core.cjs`),
no auth required:

```
GET https://api2.cursor.sh/updates/api/update/<platform>/<channel>/<current version>/<machine id>/stable
  platform: darwin-arm64 | darwin-x64 | linux-x64 | linux-arm64 | win32-x64-user | win32-arm64-user
  channel:  sand (stable) | sand-nightly | sand-dogfood
→ 204 if <current version> is newest, else
  darwin:      {"url": ".../darwin-arm64/<v>/Grok_Bot_<v>.zip", "name": "<v>"}
  linux/win32: {"url": ".../Grok_Bot_<v>.AppImage" | "..._Setup.exe", "version", "sha256hash"?}
```

It always picks the macOS build: `app.asar` is platform-neutral JavaScript, and the macOS
artifact is a plain zip (Linux ships AppImage/squashfs, Windows a Setup.exe). The zip is
read through HTTP range requests (`downloads.cursor.com` supports them; it rejects the
default `Python-urllib` user agent), so only the central directory, `app.asar` and
`Info.plist` are transferred: ~19 MB of the ~140 MB archive for 0.75.0. Output:
`.cache/app/<version>/Contents/{Info.plist,Resources/app.asar}`, which `extract_schema.py
--app` accepts like a real `.app`. For 0.75.0 the downloaded `app.asar` was byte-identical
to the installed one.

`/download/stable/<platform>/grok-bot-<id>` links on `api2.cursor.sh/updates` (the
website's download buttons) redirect to the same artifacts.

## How the extraction works

`Contents/Resources/app.asar` is an Electron archive (`tools/asar.py` reads it; format:
`[u32 4][u32 header_size][u32 payload][u32 json_len][json]`, data at `8 + header_size`).
The protobuf-es output lives in **`dist/electron-main/proto.cjs`** (if this path changes,
grep the extracted archive for `typeName:"aiserver.v1.GrokBotService"` and update `BUNDLE`).

Minification renames identifiers but keeps three patterns intact:

```js
// message: compact descriptor string + the types it references
Ab=class e extends n{ … static $(){return["SendGrokBotUserMessageResponse|1 dispatched 8|2 mode #0|4 delivery #1|5 refusal #2?",DH,oa,Y]}}
// enum (protobuf-es strips the SCREAMING_SNAKE prefix from value names)
oa=a(s,S,"GrokBotUserMessageDelivery",[[0,"UNSPECIFIED"],[1,"ACCEPTED_BOX"],…],1)
// service
_H={typeName:"aiserver.v1.GrokBotService",methods:{sendGrokBotUserMessage:{name:"SendGrokBotUserMessage",I:jq,O:_q,kind:i.Unary},…}}
```

Descriptor grammar, one field per `|`: `<number> <name> <type>[?|*] [<oneof>]`

- `<type>` is a scalar code (`1` double, `2` float, `3` int64, `4` uint64, `5` int32,
  `8` bool, `9` string, `12` bytes, `13` uint32, …), `#k` = the k-th reference after the
  string, or `K,V` = `map<K,V>`; a reference to `e` is the class itself (recursion).
- `?` = proto3 `optional`, `*` = `repeated`, a 4th token names the `oneof`.
- Google well-known types are recognised by `typeName="google.protobuf.X"` in their class.

The extractor binds each minified identifier to its message/enum name, then resolves
references. Known lossy spots:

- Nested types are flattened (`Outer.Inner` → `Outer_Inner`) into one package.
- Enum value prefixes are re-derived from the type name. They match the server for
  top-level enums (verified: `GROK_BOT_SEND_STATUS_ACCEPTED`), but code should compare by
  suffix (`api::enum_suffix`) anyway.
- One name exists as both a message and an enum in different packages (`SubagentType`);
  the message is skipped with a warning.

## Reading the app's behaviour

The schema says *what* can be sent, not *what the app sends*. For that, extract the
archive and read `dist/electron-main/main-app.cjs`:

```sh
python3 -I tools/asar.py extract .cache/app/<version>/Contents/Resources/app.asar /tmp/gb   # or the installed app's
grep -o '.\{600\}sendGrokBotUserMessage(.\{900\}' /tmp/gb/dist/electron-main/main-app.cjs
```

Client calls use the camelCase method name (`e.sendGrokBotUserMessage({...})`); request
fields appear in camelCase. Useful anchors: `getSandBackendClientHeaders` / `function VTe`
(headers), `loginDeepControl`, `/auth/poll`, `/oauth/token` (auth), `sand-secrets.json`
(credential storage).

Then try it live with the raw RPC command, which prints exact request/response JSON:

```sh
grokbot -v rpc GrokBotService/ListGrokBotAgentStatuses '{"agentIds":["<uuid>"]}'
```

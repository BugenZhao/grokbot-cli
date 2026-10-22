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
just update-schema                # tools/update-schema.sh: extract → protoc → used-RPC check → diff summary
```

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
python3 -I tools/asar.py extract "/Applications/Grok Bot.app/Contents/Resources/app.asar" /tmp/gb
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

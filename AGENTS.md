# AGENTS.md

Guidance for coding agents maintaining this repository. Read this first, then the doc for
the area you touch (see “Where things are documented”).

## What this is

`grokbot` is a non-interactive Rust CLI for Grok Bot. It calls the **private, undocumented**
backend the Grok Bot desktop app uses (Cursor's `aiserver.v1` Connect API, JSON codec).
Upstream can change at any time without notice, so most maintenance is: notice that
upstream changed, find out how the app does it now, adapt the code, and **record what you
learned in the docs**.

## Commands

```sh
just build              # debug build, code-signed (keeps keychain grants across rebuilds)
just install            # release build, signed, into ~/.cargo/bin
just lint               # cargo fmt --check + clippy -D warnings (must pass before committing)
just update-schema      # re-extract schema (installed app, or download latest; any OS)
just update-schema --download
just check-rpcs         # every RPC used in src/ still exists in schema/aiserver.proto
grokbot -v rpc GrokBotService/<Method> '<json>'   # raw call; prints request + response
```

Signing needs a code-signing identity (`GROKBOT_SIGN_IDENTITY`, default
"Apple Development"). On a machine without one, use `cargo build`; only the keychain
prompt behaviour differs (docs/auth.md).

## Layout

```
src/connect.rs     Connect client, client headers, CLIENT_VERSION
src/auth/          own PKCE login + refresh (login.rs), read-only app import (app.rs)
src/api.rs         serde types: ONLY the fields we use, protobuf-JSON conventions
src/transcript.rs  decoding of transcript entry bodies (JSON, not in the schema)
src/cmd/*.rs       one module per command group
src/ctx.rs         config, agent resolution, exit codes, table/time helpers
tools/             fetch_app.py, asar.py, extract_schema.py, check_rpcs.py, update-schema.sh
schema/            GENERATED, committed: aiserver.proto, services.txt, used.proto
docs/              see below
```

Never hand-edit files under `schema/`; change the generator in `tools/` and regenerate.

## Testing against the live service

There are no offline tests for the API layer; verify behaviour against the real backend.

- Use **only** the dedicated test bot **CLI Test**
  (`bafcb711-851d-4a63-944e-dbc9784bb9bd`): `export GROKBOT_AGENT=bafcb711`.
  Messages there are free; its routine “CLI 测试 routine” exists for toggle tests.
- Read-only calls on other bots (`agents`, `status`, `routines`, `log --json` for
  *shapes*) are fine when needed to understand formats, but **never** send messages,
  interrupt, or toggle routines on any other bot, and do not paste their message contents
  into commits, docs or chat. Describe structures with values elided.
- Prefix test messages with `[cli test …]` so they are recognisable in the transcript.
- Auth for testing: `grokbot login --from-app` (needs the desktop app signed in). Isolate
  test config with `XDG_CONFIG_HOME=<tmp dir>` when experimenting with auth.
- A minimal regression pass after any change to request/response handling:
  `agents`, `status`, `ask "<text>"`, `echo … | ask --json`, `ask --no-wait` + `wait`,
  `ask -f <file>`, `log -n 10`, `routines`, and the change-specific path.

## Safety rules (do not break)

- Never print, log, commit or paste access/refresh tokens, the keychain password, or the
  contents of `sand-secrets.json`. `-v` logs bodies, not headers; keep it that way.
- `--from-app` mode must stay read-only: never write the app's files and never use the
  app's refresh token (rotation would sign the app out). See docs/auth.md.
- Do not add commands that act on things the user did not ask for (e.g. auto-answering
  widgets, approving drafts, sending secrets).

## When upstream changes (the important part)

Symptoms: an RPC returns `not_found`/`unimplemented`/`invalid_argument`, a decode error
in `api.rs`, `ask` hangs or never sees replies, `log` shows nothing, `check-rpcs` reports
`MISSING`, or `update-schema` shows a diff in `schema/used.proto`.

1. **Reproduce** with `grokbot -v …` or `grokbot -v rpc …`. Read `details[].debug` in
   error bodies; it often states the reason in plain words.
2. **Refresh the schema**: `just update-schema` (or `--download`). Review the printed diff
   of `schema/used.proto` and `schema/services.txt`.
3. **See how the app does it now**: extract the asar (`tools/asar.py extract …`) and grep
   `dist/electron-main/main-app.cjs` for the camelCase method (`sendGrokBotUserMessage(`)
   or nearby strings; docs/schema.md lists useful anchors. Prefer copying the app's
   actual request over guessing from the schema.
4. **Explore alternatives** with `grokbot rpc` on the test bot when a method disappears
   or stops working (example: routines moved from `ListGrokBotAgentRoutines` to
   automations RPCs).
5. **Fix** the code. If the extraction itself broke (bundle path, minified patterns),
   fix `tools/extract_schema.py` rather than working around it, and make it emit a
   warning for anything it cannot resolve.
6. **Bump** `CLIENT_VERSION` in `src/connect.rs` to the app version you extracted from.
7. **Update the docs in the same change** (next section). A fix without the matching doc
   update is incomplete.
8. `just lint`, run the regression pass on the test bot, commit.

## Keeping docs current

Docs are the maintenance memory of this project: the next agent relies on them to know
what the backend does. Whenever you learn or change something, update the matching file:

| You changed / discovered | Update |
|---|---|
| A command's RPC sequence, fields sent, or completion logic | docs/commands.md |
| Transcript entry kinds/body fields, turn/settlement semantics | docs/transcript.md (+ `src/transcript.rs`) |
| Headers, checksum, codec, error format, endpoints | docs/protocol.md |
| Login/refresh flow, app credential storage, keychain behaviour | docs/auth.md |
| Extraction, bundle layout, download feed, update procedure | docs/schema.md |
| User-visible flags, commands, exit codes, config | README.md (+ clap help text) |
| Repo workflow, test bot, rules for agents | AGENTS.md |
| **Any upstream behaviour change or surprise** | **docs/upstream-log.md** (append an entry) |

Rules:

- Write facts you verified, and say how (“observed on 0.75.0”, “verified by …”). Mark
  unverified assumptions explicitly as such.
- Remove or correct statements that are no longer true instead of appending caveats.
- Keep docs in English, concise, with exact RPC/field names so they are greppable.
- Mention the Grok Bot version when a behaviour is version-specific.

## Conventions

- Match the surrounding code: small modules, `anyhow` errors, `ctx::Exit` for errors
  with dedicated exit codes (README lists them; keep the list in sync).
- `api.rs` declares only fields that are used, all `#[serde(default)]`, int64 as strings
  (`i64_str`), enums compared by suffix (`enum_suffix`), never exhaustive enums.
- stdout carries results only; progress/hints go to stderr via `ctx.note`; every
  command supports `--json` (one object per line).
- Commit messages: imperative summary line, a short body explaining why.

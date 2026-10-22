# grokbot

A non-interactive CLI for [Grok Bot](https://x.ai/bot): message your bots from scripts,
read transcripts, check status, and toggle routines.

It talks to the same private backend as the desktop app (Cursor's `aiserver.v1` Connect
API). That API is undocumented and may change at any time; `docs/schema.md` explains how
to catch up when it does.

## Install

Prebuilt binaries for macOS (Apple silicon) and Linux (x86_64, arm64) are attached to
each [GitHub release](../../releases) as `grokbot-<version>-<target>.tar.gz` with a
`.sha256` file. The macOS binary is ad-hoc signed and **not notarized**: if you download it
with a browser, clear the quarantine flag first (`xattr -d com.apple.quarantine grokbot`);
`curl` downloads are not affected.

From source:

```sh
just install     # release build, signed with a stable identity, into ~/.cargo/bin
# or: cargo install --path .   (unsigned: macOS re-asks for keychain access after each rebuild)
```

`just` lists the other recipes (`build`, `lint`, `test`, `smoke`, `update-schema`, `check-rpcs`).

## Sign in

```sh
grokbot login             # own session via browser sign-in (recommended)
grokbot login --from-app  # or borrow the Grok Bot desktop app's session (read-only, macOS only)
grokbot whoami
```

See [docs/auth.md](docs/auth.md) for the trade-offs.

## Usage

```sh
grokbot agents                                  # list bots
grokbot ask -a research "summarise today's HN top 5"
echo "what's in this log?" | grokbot ask -a ops -f app.log
id=$(grokbot ask -a ops --no-wait "run the weekly report")
grokbot wait -a ops "$id" --timeout 30m
grokbot log -a ops -n 50            # transcript; --follow to keep polling, --all for internals
grokbot status                      # running / idle / awaiting you, for every bot
grokbot stop -a ops                 # interrupt the current turn
grokbot tasks -a ops [--done]       # background work items; --checklist for the session to-dos
grokbot routines                    # scheduled routines of all bots
grokbot routines disable <id>       # id or unique prefix, as listed
grokbot rpc GrokBotService/ListGrokBotAgents '{}'   # raw RPC for exploring
```

`-a` accepts a bot UUID, its name, or a unique prefix (`GROKBOT_AGENT` works too).

Output: replies go to **stdout**, progress and hints to **stderr** (`-q` silences them).
`--json` prints one JSON object per line on every command.

### Exit codes

| code | meaning |
|---|---|
| 0 | ok |
| 1 | error (network, server, turn ended with outcome ≠ success) |
| 2 | usage error, or bot name matched none/several |
| 3 | not signed in / session expired |
| 4 | message or upload refused by the server |
| 5 | `--timeout` reached; the bot may still be working (`grokbot wait` resumes) |
| 6 | the bot is waiting for input the CLI cannot give (buttons, secret, draft approval); open the app |

### Configuration

`~/.config/grokbot/config.toml` (or `profiles/NAME/config.toml` with `--profile NAME`):

```toml
timeout = "10m"        # default for ask/wait
poll_interval = "1s"
```

Environment: `GROKBOT_AGENT`, `GROKBOT_PROFILE`, `GROKBOT_TOKEN` (raw access token, for CI).

## Documentation

- [docs/commands.md](docs/commands.md) — every command's exact RPC sequence
- [docs/transcript.md](docs/transcript.md) — transcript entry formats and the turn model
- [docs/protocol.md](docs/protocol.md) — Connect transport, headers, errors
- [docs/auth.md](docs/auth.md) — login flows and the app's credential storage
- [docs/schema.md](docs/schema.md) — how the schema is extracted and how to update it
- [docs/upstream-log.md](docs/upstream-log.md) — observed upstream behaviour and changes, by version
- [AGENTS.md](AGENTS.md) — maintenance guide for coding agents

## Layout

```
src/connect.rs     Connect JSON client + client headers
src/auth/          own login (PKCE/refresh) and read-only app import
src/api.rs         serde types for the RPCs we use
src/transcript.rs  transcript body decoding
src/cmd/           one module per command group
tools/             app downloader, asar reader, schema extractor, used-RPC checker, update script,
                   end-to-end smoke test
justfile           build/install with code signing, lint, schema update
schema/            generated: aiserver.proto, services.txt, used.proto
```

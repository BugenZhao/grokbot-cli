# Signing identity and the stable code-signing identifier. Keychain "Always Allow"
# grants are tied to this identifier + certificate, so they survive rebuilds
# (see docs/auth.md). Override via GROKBOT_SIGN_IDENTITY / GROKBOT_SIGN_ID.
identity := env("GROKBOT_SIGN_IDENTITY", "Apple Development")
bundle_id := env("GROKBOT_SIGN_ID", "com.bugenzhao.grokbot")
bin_dir := env("CARGO_HOME", home_directory() / ".cargo") / "bin"

default:
    @just --list

# Debug build, signed so `--from-app` keychain grants persist across rebuilds.
build:
    cargo build
    just sign target/debug/grokbot

# Release build, signed, installed to ~/.cargo/bin.
install:
    cargo build --release
    just sign target/release/grokbot
    install -m 755 target/release/grokbot "{{bin_dir}}/grokbot"
    @echo "installed {{bin_dir}}/grokbot"

# Re-sign a binary with the stable identity (default: the release binary).
sign path="target/release/grokbot":
    codesign --force --sign "{{identity}}" --identifier {{bundle_id}} "{{path}}"
    @codesign -dv "{{path}}" 2>&1 | grep -E '^(Identifier|Authority=Apple Development|Authority=Developer ID|TeamIdentifier)' || true

lint:
    cargo fmt --check
    cargo clippy -- -D warnings

# Re-extract the schema and summarise what changed. No argument: the installed app if
# present, else download the latest build. Or pass `--download`, or a path to the .app.
[positional-arguments]
update-schema *args:
    tools/update-schema.sh "$@"

# Check that every RPC the CLI calls still exists in schema/aiserver.proto.
check-rpcs:
    python3 -I tools/check_rpcs.py

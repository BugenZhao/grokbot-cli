#!/bin/sh
# Re-extract the schema from the installed Grok Bot app and show what changed for us.
# Usage: tools/update-schema.sh [path/to/Grok Bot.app]
set -eu
cd "$(dirname "$0")/.."
APP="${1:-/Applications/Grok Bot.app}"

python3 -I tools/extract_schema.py --app "$APP"
(cd schema && protoc --descriptor_set_out=/dev/null aiserver.proto) || {
    echo "warning: protoc rejects the regenerated schema; fix tools/extract_schema.py" >&2
}
status=0
python3 -I tools/check_rpcs.py --write || status=$?

echo
echo "== RPCs added/removed (all services) =="
git diff --no-color -U0 -- schema/services.txt | grep -E '^[+-][A-Za-z]' || echo "(none)"
echo
echo "== Changes affecting the CLI (schema/used.proto) =="
git diff --stat -- schema/used.proto
git diff --no-color -- schema/used.proto | grep -E '^[+-] ' || echo "(none)"
echo
echo "App version: $(plutil -extract CFBundleShortVersionString raw "$APP/Contents/Info.plist")" \
     "— remember to bump CLIENT_VERSION in src/connect.rs."
exit $status

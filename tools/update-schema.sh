#!/bin/sh
# Re-extract the schema from Grok Bot and show what changed for us.
#
#   tools/update-schema.sh                 # installed app if present, else download latest
#   tools/update-schema.sh --download      # always download the latest stable build
#   tools/update-schema.sh "/path/Grok Bot.app"
#
# Works on any OS with python3 (protoc is optional). Downloads go to .cache/app/<version>.
set -eu
cd "$(dirname "$0")/.."

case "${1:-}" in
    --download) APP=$(python3 -I tools/fetch_app.py --out .cache/app) ;;
    "")
        if [ -f "/Applications/Grok Bot.app/Contents/Resources/app.asar" ]; then
            APP="/Applications/Grok Bot.app"
        else
            APP=$(python3 -I tools/fetch_app.py --out .cache/app)
        fi
        ;;
    *) APP="$1" ;;
esac
echo "using: $APP" >&2

python3 -I tools/extract_schema.py --app "$APP"
if command -v protoc >/dev/null 2>&1; then
    (cd schema && protoc --descriptor_set_out=/dev/null aiserver.proto) || {
        echo "warning: protoc rejects the regenerated schema; fix tools/extract_schema.py" >&2
    }
else
    echo "note: protoc not found; skipping the .proto validity check" >&2
fi
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
VERSION=$(python3 -I -c 'import plistlib,sys; print(plistlib.load(open(sys.argv[1] + "/Contents/Info.plist", "rb"))["CFBundleShortVersionString"])' "$APP")
echo "App version: $VERSION — remember to bump CLIENT_VERSION in src/connect.rs."
exit $status

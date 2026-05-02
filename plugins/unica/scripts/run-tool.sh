#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -lt 1 ]; then
  echo "usage: run-tool.sh <tool-name> [args...]" >&2
  exit 64
fi

TOOL_NAME="$1"
shift

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PLUGIN_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
MANIFEST="$PLUGIN_ROOT/third-party/manifest.json"

HOST_OS="$(uname -s)"
HOST_ARCH="$(uname -m)"

case "${HOST_OS}-${HOST_ARCH}" in
  Darwin-arm64) TARGET_ID="darwin-arm64" ;;
  Linux-x86_64) TARGET_ID="linux-x64" ;;
  Linux-amd64) TARGET_ID="linux-x64" ;;
  *)
    echo "Unica does not ship binaries for ${HOST_OS}-${HOST_ARCH}." >&2
    exit 78
    ;;
esac

if ! command -v python3 >/dev/null 2>&1; then
  echo "python3 is required to read Unica third-party manifest." >&2
  exit 69
fi

if [ ! -f "$MANIFEST" ]; then
  echo "Unica third-party manifest not found: $MANIFEST" >&2
  exit 66
fi

TOOL_METADATA="$(python3 - "$MANIFEST" "$TOOL_NAME" "$TARGET_ID" <<'PY'
import json
import sys
from pathlib import Path

manifest_path = Path(sys.argv[1])
tool_name = sys.argv[2]
target_id = sys.argv[3]
manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
for tool in manifest.get("tools", []):
    if tool.get("name") != tool_name:
        continue

    binaries = tool.get("binaries")
    if binaries:
        binary = binaries.get(target_id)
        if not binary:
            supported = ", ".join(sorted(binaries))
            print(f"tool {tool_name} is not packaged for {target_id}; supported: {supported}", file=sys.stderr)
            sys.exit(78)
        print(binary["binaryPath"])
        print(binary["sha256"])
        sys.exit(0)

    if manifest.get("targetTriple") and target_id != "darwin-arm64":
        print(f"legacy manifest only supports darwin-arm64; current target is {target_id}", file=sys.stderr)
        sys.exit(78)
    print(tool["binaryPath"])
    print(tool["sha256"])
    sys.exit(0)

print(f"tool not found in manifest: {tool_name}", file=sys.stderr)
sys.exit(1)
PY
)"

BINARY_RELATIVE="$(printf '%s\n' "$TOOL_METADATA" | sed -n '1p')"
EXPECTED_SHA="$(printf '%s\n' "$TOOL_METADATA" | sed -n '2p')"
BINARY="$PLUGIN_ROOT/$BINARY_RELATIVE"

if [ ! -x "$BINARY" ]; then
  echo "Unica binary is missing or not executable: $BINARY" >&2
  exit 66
fi

if command -v shasum >/dev/null 2>&1; then
  ACTUAL_SHA="$(shasum -a 256 "$BINARY" | awk '{print $1}')"
elif command -v sha256sum >/dev/null 2>&1; then
  ACTUAL_SHA="$(sha256sum "$BINARY" | awk '{print $1}')"
else
  echo "shasum or sha256sum is required to verify Unica bundled tools." >&2
  exit 69
fi
if [ "$ACTUAL_SHA" != "$EXPECTED_SHA" ]; then
  echo "Unica binary checksum mismatch for $TOOL_NAME." >&2
  echo "expected: $EXPECTED_SHA" >&2
  echo "actual:   $ACTUAL_SHA" >&2
  exit 65
fi

exec "$BINARY" "$@"

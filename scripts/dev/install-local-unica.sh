#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/dev/install-local-unica.sh [options]

Build a fully working local Unica Codex plugin for the current machine,
install it as a local Codex marketplace, and verify fresh-session visibility.

Options:
  --marketplace-name NAME  Codex marketplace name (default: unica-dev)
  --build-dir DIR         Build directory (default: .build/local-codex-unica)
  --skip-build            Reuse an existing target tool bundle in --build-dir
  --skip-install          Build/package only, do not modify Codex config/cache
  --skip-verify           Do not run codex debug prompt-input verification
  -h, --help              Show this help
EOF
}

target_for_host() {
  local host_os="$1"
  local host_arch="$2"
  case "${host_os}-${host_arch}" in
    Darwin-arm64) printf '%s\n' "darwin-arm64" ;;
    Linux-x86_64|Linux-amd64) printf '%s\n' "linux-x64" ;;
    MINGW*_NT-*-x86_64|MINGW*_NT-*-amd64) printf '%s\n' "win-x64" ;;
    *)
      echo "Unsupported local Unica tool target: ${host_os}-${host_arch}" >&2
      return 78
      ;;
  esac
}

detect_target() {
  target_for_host "$(uname -s)" "$(uname -m)"
}

main() {
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MARKETPLACE_NAME="${UNICA_CODEX_MARKETPLACE_NAME:-unica-dev}"
BUILD_ROOT="${UNICA_LOCAL_BUILD_DIR:-$REPO_ROOT/.build/local-codex-unica}"
DO_BUILD=1
DO_INSTALL=1
DO_VERIFY=1

while [ "$#" -gt 0 ]; do
  case "$1" in
    --marketplace-name)
      MARKETPLACE_NAME="${2:?missing value for --marketplace-name}"
      shift 2
      ;;
    --build-dir)
      BUILD_ROOT="${2:?missing value for --build-dir}"
      shift 2
      ;;
    --skip-build)
      DO_BUILD=0
      shift
      ;;
    --skip-install)
      DO_INSTALL=0
      shift
      ;;
    --skip-verify)
      DO_VERIFY=0
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 64
      ;;
  esac
done

select_python() {
  for candidate in "${PYTHON:-}" python3.12 python3.11 python3.10 python3 python; do
    if [ -z "$candidate" ]; then
      continue
    fi
    if ! command -v "$candidate" >/dev/null 2>&1; then
      continue
    fi
    if "$candidate" - <<'PY' >/dev/null 2>&1
import sys
raise SystemExit(0 if sys.version_info >= (3, 10) else 1)
PY
    then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  echo "Python >= 3.10 is required. Set PYTHON=/path/to/python if needed." >&2
  return 69
}

tool_binary() {
  local tool="$1"
  local suffix=""
  if [ "$TARGET" = "win-x64" ]; then
    suffix=".exe"
  fi
  printf '%s\n' "$MARKETPLACE_DIR/plugins/unica/bin/$TARGET/${tool}${suffix}"
}

PYTHON_BIN="$(select_python)"
TARGET="$(detect_target)"
BUILD_ROOT="$(cd "$REPO_ROOT" && mkdir -p "$BUILD_ROOT" && cd "$BUILD_ROOT" && pwd)"
TOOLS_ROOT="$BUILD_ROOT/tool-artifacts"
TOOL_BUNDLE="${UNICA_LOCAL_TOOL_BUNDLE:-$TOOLS_ROOT/unica-tools-$TARGET}"
WORK_DIR="$BUILD_ROOT/tool-work/$TARGET"
PACKAGE_OUT="$BUILD_ROOT/package"
MARKETPLACE_DIR="$PACKAGE_OUT/marketplace"
PROMPT_PROOF="$BUILD_ROOT/prompt-input.json"
CODEX_HOME_DIR="${CODEX_HOME:-$HOME/.codex}"
CODEX_CONFIG="$CODEX_HOME_DIR/config.toml"
CODEX_PLUGIN_CACHE_DIR="$CODEX_HOME_DIR/plugins/cache/$MARKETPLACE_NAME/unica"

enable_codex_plugin() {
  local plugin_key="unica@$MARKETPLACE_NAME"
  "$PYTHON_BIN" - "$CODEX_CONFIG" "$plugin_key" <<'PY'
from __future__ import annotations

import sys
from pathlib import Path

config_path = Path(sys.argv[1]).expanduser()
plugin_key = sys.argv[2]
table = f'[plugins."{plugin_key}"]'

config_path.parent.mkdir(parents=True, exist_ok=True)
text = config_path.read_text(encoding="utf-8") if config_path.exists() else ""
lines = text.splitlines()
out: list[str] = []
found = False
i = 0

while i < len(lines):
    line = lines[i]
    if line.strip() == table:
        found = True
        out.append(line)
        i += 1
        retained: list[str] = []
        while i < len(lines) and not lines[i].lstrip().startswith("["):
            if not lines[i].strip().startswith("enabled"):
                retained.append(lines[i])
            i += 1
        out.append("enabled = true")
        out.extend(retained)
        continue
    out.append(line)
    i += 1

if not found:
    if out and out[-1].strip():
        out.append("")
    out.append(table)
    out.append("enabled = true")

config_path.write_text("\n".join(out).rstrip() + "\n", encoding="utf-8")
PY
}

cd "$REPO_ROOT"

echo "==> Unica local target: $TARGET"
echo "==> Build root: $BUILD_ROOT"
echo "==> Marketplace: $MARKETPLACE_NAME"

if [ "$DO_BUILD" -eq 1 ]; then
  rm -rf "$TOOL_BUNDLE" "$WORK_DIR"
  "$PYTHON_BIN" scripts/ci/build-unica-tools.py \
    --target "$TARGET" \
    --lock-file plugins/unica/third-party/tools.lock.json \
    --out-dir "$TOOL_BUNDLE" \
    --work-dir "$WORK_DIR"
else
  if [ ! -f "$TOOL_BUNDLE/tools.json" ]; then
    echo "--skip-build requested, but bundle is missing: $TOOL_BUNDLE/tools.json" >&2
    exit 66
  fi
fi

rm -rf "$PACKAGE_OUT"
"$PYTHON_BIN" scripts/ci/package-unica-plugin.py \
  --repo-root "$REPO_ROOT" \
  --tools-root "$TOOLS_ROOT" \
  --lock-file plugins/unica/third-party/tools.lock.json \
  --out-dir "$PACKAGE_OUT" \
  --marketplace-name "$MARKETPLACE_NAME" \
  --local-debug-target "$TARGET"

"$PYTHON_BIN" -m json.tool "$MARKETPLACE_DIR/.agents/plugins/marketplace.json" >/dev/null
"$PYTHON_BIN" -m json.tool "$MARKETPLACE_DIR/plugins/unica/.codex-plugin/plugin.json" >/dev/null
"$PYTHON_BIN" -m json.tool "$MARKETPLACE_DIR/plugins/unica/.mcp.json" >/dev/null
"$PYTHON_BIN" -m json.tool "$MARKETPLACE_DIR/plugins/unica/third-party/manifest.json" >/dev/null

"$(tool_binary v8-runner)" config init --help >/dev/null
"$(tool_binary unica)" --help >/dev/null
PLUGIN_VERSION="$("$PYTHON_BIN" -c 'import json, sys; print(json.load(open(sys.argv[1], encoding="utf-8"))["version"])' "$MARKETPLACE_DIR/plugins/unica/.codex-plugin/plugin.json")"
CODEX_PLUGIN_CACHE_VERSION_DIR="$CODEX_PLUGIN_CACHE_DIR/$PLUGIN_VERSION"

if [ "$DO_INSTALL" -eq 1 ]; then
  if ! command -v codex >/dev/null 2>&1; then
    echo "codex CLI is required for install. Re-run with --skip-install to build only." >&2
    exit 69
  fi

  codex plugin marketplace remove "$MARKETPLACE_NAME" >/dev/null 2>&1 || true
  if [ -d "$CODEX_PLUGIN_CACHE_DIR" ]; then
    echo "==> Removing stale Codex plugin cache: $CODEX_PLUGIN_CACHE_DIR"
    rm -rf "$CODEX_PLUGIN_CACHE_DIR"
  fi
  codex plugin marketplace add "$MARKETPLACE_DIR"
  mkdir -p "$CODEX_PLUGIN_CACHE_DIR"
  cp -R "$MARKETPLACE_DIR/plugins/unica" "$CODEX_PLUGIN_CACHE_VERSION_DIR"
  enable_codex_plugin
  echo "==> Installed Unica plugin cache: $CODEX_PLUGIN_CACHE_VERSION_DIR"

  if [ "$DO_VERIFY" -eq 1 ]; then
    codex debug prompt-input 'test' > "$PROMPT_PROOF"
    for needle in "Unica" "v8-runner" "db-auth-check"; do
      if ! grep -q "$needle" "$PROMPT_PROOF"; then
        echo "Codex prompt verification did not contain '$needle'." >&2
        echo "Saved prompt proof: $PROMPT_PROOF" >&2
        exit 65
      fi
    done
  fi
fi

echo "==> Local Unica marketplace ready: $MARKETPLACE_DIR"
if [ "$DO_INSTALL" -eq 1 ]; then
  echo "==> Installed in Codex as marketplace '$MARKETPLACE_NAME'"
  if [ "$DO_VERIFY" -eq 1 ]; then
    echo "==> Fresh prompt proof: $PROMPT_PROOF"
  fi
fi
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi

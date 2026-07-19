#!/usr/bin/env sh
set -eu

usage() {
  cat <<'EOF'
Usage: install-unica.sh [options]

Migrate Unica to the public IngvarConsulting/unica-marketplace catalog using
Git and the native transactional bootstrap. The bootstrap reports and retains
the migration backup used for automatic rollback.

Options:
  --ref REF         Frozen marketplace snapshot used for migration (default: v0.7.8)
  --target TARGET   Override host target: darwin-arm64 or linux-x64
  --codex-home DIR  Codex home directory (default: $CODEX_HOME or ~/.codex)
  -h, --help        Show this help
EOF
}

MARKETPLACE_REPOSITORY="https://github.com/IngvarConsulting/unica-marketplace.git"
MARKETPLACE_REF="${UNICA_MARKETPLACE_REF:-v0.7.8}"
TARGET="${UNICA_TARGET:-}"
CODEX_HOME_DIR="${CODEX_HOME:-}"

while [ "$#" -gt 0 ]; do
  case "$1" in
    --ref)
      MARKETPLACE_REF="${2:?missing value for --ref}"
      shift 2
      ;;
    --target)
      TARGET="${2:?missing value for --target}"
      shift 2
      ;;
    --codex-home)
      CODEX_HOME_DIR="${2:?missing value for --codex-home}"
      shift 2
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

detect_target() {
  host_os="$(uname -s)"
  host_arch="$(uname -m)"
  case "${host_os}-${host_arch}" in
    Darwin-arm64|Darwin-aarch64) printf '%s\n' "darwin-arm64" ;;
    Linux-x86_64|Linux-amd64) printf '%s\n' "linux-x64" ;;
    *)
      echo "Unsupported Unica host: ${host_os}-${host_arch}" >&2
      exit 78
      ;;
  esac
}

default_codex_home() {
  if [ -n "${HOME:-}" ]; then
    printf '%s\n' "$HOME/.codex"
  elif [ -n "${USERPROFILE:-}" ]; then
    printf '%s\n' "$USERPROFILE/.codex"
  else
    echo "CODEX_HOME, HOME, or USERPROFILE is required." >&2
    exit 78
  fi
}

case "$MARKETPLACE_REF" in
  ""|*[!A-Za-z0-9._/-]*)
    echo "Unsafe marketplace ref: $MARKETPLACE_REF" >&2
    exit 64
    ;;
esac

if ! command -v git >/dev/null 2>&1; then
  echo "Git is required to install or migrate Unica." >&2
  exit 69
fi
if ! command -v codex >/dev/null 2>&1; then
  echo "Codex CLI is required to install or migrate Unica." >&2
  exit 69
fi
git -c 'alias.unica-probe=!f() { exit 0; }; f' unica-probe

TARGET="${TARGET:-$(detect_target)}"
case "$TARGET" in
  darwin-arm64|linux-x64) ;;
  *)
    echo "Unsupported Unica target: $TARGET" >&2
    exit 78
    ;;
esac

if [ -z "$CODEX_HOME_DIR" ]; then
  CODEX_HOME_DIR="$(default_codex_home)"
fi
export CODEX_HOME="$CODEX_HOME_DIR"

TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/unica-migrate.XXXXXX")"
trap 'rm -rf "$TMP_ROOT"' EXIT INT TERM
MARKETPLACE_DIR="$TMP_ROOT/marketplace"

git clone --depth 1 --branch "$MARKETPLACE_REF" "$MARKETPLACE_REPOSITORY" "$MARKETPLACE_DIR"
CATALOG="$MARKETPLACE_DIR/.agents/plugins/marketplace.json"
if [ ! -f "$CATALOG" ]; then
  echo "Stable marketplace catalog is missing: $CATALOG" >&2
  exit 65
fi

PINNED_REF="$(sed -n 's/.*"ref"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$CATALOG" | head -n 1)"
case "$PINNED_REF" in
  v[0-9A-Za-z._-]*) ;;
  *)
    echo "Marketplace catalog does not contain an immutable Unica tag." >&2
    exit 65
    ;;
esac

git -C "$MARKETPLACE_DIR" fetch --depth 1 origin "refs/tags/$PINNED_REF:refs/tags/$PINNED_REF"
git -C "$MARKETPLACE_DIR" checkout --detach "$PINNED_REF"

PLUGIN_ROOT="$MARKETPLACE_DIR/plugins/unica"
BOOTSTRAP="$PLUGIN_ROOT/bootstrap/bin/$TARGET/unica-bootstrap"
if [ ! -x "$BOOTSTRAP" ]; then
  echo "Native Unica bootstrap is missing or not executable: $BOOTSTRAP" >&2
  exit 66
fi

echo "==> Preflight Unica migration from $PINNED_REF"
"$BOOTSTRAP" migrate-preflight --plugin-root "$PLUGIN_ROOT" --marketplace-ref "$MARKETPLACE_REF"
echo "==> Apply transactional Unica migration"
MIGRATION_OUTPUT="$("$BOOTSTRAP" migrate --plugin-root "$PLUGIN_ROOT" --marketplace-ref "$MARKETPLACE_REF")"
printf '%s\n' "$MIGRATION_OUTPUT"
BACKUP_DIR="$(printf '%s\n' "$MIGRATION_OUTPUT" | sed -n 's/.*"backupDir"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)"
if [ -n "$BACKUP_DIR" ]; then
  echo "==> Migration backup: $BACKUP_DIR"
else
  echo "==> Migration backup: not required (already canonical)"
fi
echo "==> Open a new Codex task or restart the client to use Unica $PINNED_REF"

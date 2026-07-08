#!/usr/bin/env sh
set -eu

usage() {
  cat <<'EOF'
Usage: install-unica.sh [options]

Download the Unica package for the current platform from GitHub Releases,
install it into Codex, and verify fresh-session visibility.

Options:
  --version VERSION       Release tag to install, for example v0.6.1 (default: latest)
  --target TARGET         Override detected target: darwin-arm64 or linux-x64
  --marketplace-name NAME Codex marketplace name (default: unica-local)
  --codex-home DIR        Codex home directory (default: $CODEX_HOME or ~/.codex)
  --skip-verify           Do not run codex debug prompt-input verification
  --print-download-url    Print the resolved release asset URL and exit
  -h, --help              Show this help
EOF
}

REPO="${UNICA_REPO:-IngvarConsulting/unica}"
VERSION="${UNICA_VERSION:-latest}"
TARGET="${UNICA_TARGET:-}"
MARKETPLACE_NAME="${UNICA_CODEX_MARKETPLACE_NAME:-unica-local}"
CODEX_HOME_DIR="${CODEX_HOME:-}"
DO_VERIFY=1
PRINT_DOWNLOAD_URL=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version)
      VERSION="${2:?missing value for --version}"
      shift 2
      ;;
    --target)
      TARGET="${2:?missing value for --target}"
      shift 2
      ;;
    --marketplace-name)
      MARKETPLACE_NAME="${2:?missing value for --marketplace-name}"
      shift 2
      ;;
    --codex-home)
      CODEX_HOME_DIR="${2:?missing value for --codex-home}"
      shift 2
      ;;
    --skip-verify)
      DO_VERIFY=0
      shift
      ;;
    --print-download-url)
      PRINT_DOWNLOAD_URL=1
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

detect_target() {
  host_os="$(uname -s)"
  host_arch="$(uname -m)"
  case "${host_os}-${host_arch}" in
    Darwin-arm64|Darwin-aarch64) printf '%s\n' "darwin-arm64" ;;
    Linux-x86_64|Linux-amd64) printf '%s\n' "linux-x64" ;;
    *)
      echo "Unsupported Unica release target for host: ${host_os}-${host_arch}" >&2
      exit 78
      ;;
  esac
}

archive_extension() {
  case "$1" in
    darwin-arm64|linux-x64) printf '%s\n' "tar.gz" ;;
    *)
      echo "Unsupported Unica release target: $1" >&2
      exit 78
      ;;
  esac
}

tool_binary() {
  tool="$1"
  printf '%s\n' "$MARKETPLACE_DIR/plugins/unica/bin/$TARGET/${tool}"
}

default_codex_home() {
  if [ -n "${HOME:-}" ]; then
    printf '%s\n' "$HOME/.codex"
  elif [ -n "${USERPROFILE:-}" ]; then
    printf '%s\n' "$USERPROFILE/.codex"
  else
    echo "CODEX_HOME, HOME, or USERPROFILE is required to install Unica." >&2
    exit 78
  fi
}

release_asset_url() {
  target="$1"
  version="$2"
  ext="$(archive_extension "$target")"
  asset="unica-codex-marketplace-${target}.${ext}"
  if [ "$version" = "latest" ]; then
    printf 'https://github.com/%s/releases/latest/download/%s\n' "$REPO" "$asset"
  else
    printf 'https://github.com/%s/releases/download/%s/%s\n' "$REPO" "$version" "$asset"
  fi
}

download_file() {
  url="$1"
  dest="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fL "$url" -o "$dest"
  elif command -v wget >/dev/null 2>&1; then
    wget -O "$dest" "$url"
  else
    echo "curl or wget is required to download Unica release assets." >&2
    exit 69
  fi
}

extract_archive() {
  archive="$1"
  dest="$2"
  case "$archive" in
    *.tar.gz)
      tar -xzf "$archive" -C "$dest"
      ;;
    *.zip)
      if command -v unzip >/dev/null 2>&1; then
        unzip -q "$archive" -d "$dest"
      else
        echo "unzip is required to extract $archive." >&2
        exit 69
      fi
      ;;
    *)
      echo "Unsupported archive type: $archive" >&2
      exit 78
      ;;
  esac
}

find_marketplace_root() {
  root="$1"
  marker="$(find "$root" -path '*/.agents/plugins/marketplace.json' -type f -print | head -n 1)"
  if [ -z "$marker" ]; then
    echo "Downloaded archive does not contain .agents/plugins/marketplace.json" >&2
    exit 65
  fi
  printf '%s\n' "${marker%/.agents/plugins/marketplace.json}"
}

read_plugin_version() {
  plugin_json="$1"
  version="$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$plugin_json" | head -n 1)"
  if [ -z "$version" ]; then
    echo "Cannot read plugin version from $plugin_json" >&2
    exit 65
  fi
  printf '%s\n' "$version"
}

enable_codex_plugin() {
  config="$CODEX_HOME_DIR/config.toml"
  table="[plugins.\"unica@$MARKETPLACE_NAME\"]"
  tmp="${config}.tmp.$$"
  mkdir -p "$(dirname "$config")"

  if [ -f "$config" ]; then
    awk -v table="$table" '
      $0 == table { skip = 1; next }
      skip && $0 ~ /^\[/ { skip = 0 }
      !skip { print }
    ' "$config" > "$tmp"
  else
    : > "$tmp"
  fi

  {
    printf '\n%s\n' "$table"
    printf 'enabled = true\n'
  } >> "$tmp"
  mv "$tmp" "$config"
}

TARGET="${TARGET:-$(detect_target)}"
URL="$(release_asset_url "$TARGET" "$VERSION")"

if [ "$PRINT_DOWNLOAD_URL" -eq 1 ]; then
  printf '%s\n' "$URL"
  exit 0
fi

if [ -z "$CODEX_HOME_DIR" ]; then
  CODEX_HOME_DIR="$(default_codex_home)"
fi

if ! command -v codex >/dev/null 2>&1; then
  echo "codex CLI is required to install Unica." >&2
  exit 69
fi

TMP_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/unica-install.XXXXXX")"
trap 'rm -rf "$TMP_ROOT"' EXIT INT TERM

ARCHIVE="$TMP_ROOT/unica-codex-marketplace-${TARGET}.$(archive_extension "$TARGET")"
EXTRACT_DIR="$TMP_ROOT/extract"
mkdir -p "$EXTRACT_DIR"

echo "==> Unica target: $TARGET"
echo "==> Download: $URL"
download_file "$URL" "$ARCHIVE"
extract_archive "$ARCHIVE" "$EXTRACT_DIR"
EXTRACTED_MARKETPLACE_DIR="$(find_marketplace_root "$EXTRACT_DIR")"
MARKETPLACE_DIR="$CODEX_HOME_DIR/marketplaces/$MARKETPLACE_NAME"
rm -rf "$MARKETPLACE_DIR"
mkdir -p "$(dirname "$MARKETPLACE_DIR")"
cp -R "$EXTRACTED_MARKETPLACE_DIR" "$MARKETPLACE_DIR"

"$(tool_binary v8-runner)" config init --help >/dev/null
"$(tool_binary unica)" --help >/dev/null
PLUGIN_VERSION="$(read_plugin_version "$MARKETPLACE_DIR/plugins/unica/.codex-plugin/plugin.json")"
CODEX_PLUGIN_CACHE_DIR="$CODEX_HOME_DIR/plugins/cache/$MARKETPLACE_NAME/unica"
CODEX_PLUGIN_CACHE_VERSION_DIR="$CODEX_PLUGIN_CACHE_DIR/$PLUGIN_VERSION"

codex plugin marketplace remove "$MARKETPLACE_NAME" >/dev/null 2>&1 || true
if [ -d "$CODEX_PLUGIN_CACHE_DIR" ]; then
  echo "==> Removing stale Codex plugin cache: $CODEX_PLUGIN_CACHE_DIR"
  rm -rf "$CODEX_PLUGIN_CACHE_DIR"
fi

codex plugin marketplace add "$MARKETPLACE_DIR"
mkdir -p "$CODEX_PLUGIN_CACHE_DIR"
cp -R "$MARKETPLACE_DIR/plugins/unica" "$CODEX_PLUGIN_CACHE_VERSION_DIR"
enable_codex_plugin

if [ "$DO_VERIFY" -eq 1 ]; then
  mkdir -p "$CODEX_HOME_DIR/tmp"
  PROMPT_PROOF="$CODEX_HOME_DIR/tmp/unica-install-prompt-input.json"
  codex debug prompt-input 'test' > "$PROMPT_PROOF"
  for needle in "$MARKETPLACE_NAME" "unica:meta-compile" "unica:v8-runner" "unica:db-auth-check"; do
    if ! grep -Fq "$needle" "$PROMPT_PROOF"; then
      echo "Codex prompt verification did not contain '$needle'." >&2
      echo "Saved prompt proof: $PROMPT_PROOF" >&2
      exit 65
    fi
  done
  echo "==> Fresh prompt proof: $PROMPT_PROOF"
fi

echo "==> Installed Unica $PLUGIN_VERSION in Codex as marketplace '$MARKETPLACE_NAME'"

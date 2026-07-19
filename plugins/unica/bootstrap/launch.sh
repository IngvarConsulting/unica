#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
  echo "usage: launch.sh <plugin-root>" >&2
  exit 64
fi

plugin_root=$1
host_os=${UNICA_BOOTSTRAP_UNAME_S:-$(uname -s)}
host_arch=${UNICA_BOOTSTRAP_UNAME_M:-$(uname -m)}

case "${host_os}-${host_arch}" in
  Darwin-arm64|Darwin-aarch64)
    target=darwin-arm64
    executable=unica-bootstrap
    ;;
  Linux-x86_64|Linux-amd64)
    target=linux-x64
    executable=unica-bootstrap
    ;;
  MINGW*-x86_64|MINGW*-amd64|MSYS*-x86_64|MSYS*-amd64|CYGWIN*-x86_64|CYGWIN*-amd64)
    target=win-x64
    executable=unica-bootstrap.exe
    ;;
  *)
    echo "Unsupported Unica host: ${host_os}-${host_arch}" >&2
    exit 78
    ;;
esac

bootstrap="${plugin_root%/}/bootstrap/bin/${target}/${executable}"
if [ ! -f "$bootstrap" ]; then
  echo "Unica bootstrap is missing for ${target}: ${bootstrap}" >&2
  exit 66
fi

exec "$bootstrap" run --plugin-root "$plugin_root"

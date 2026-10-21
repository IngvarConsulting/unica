#!/usr/bin/env bash
# Clippy гейта: `workspace` — весь workspace со всеми признаками и SARIF для
# Code Scanning, `packages` — каждый пакет со своими признаками.
#
# Сборка workspace объединяет признаки: dev-зависимость одного пакета
# включает `test-support` другого, и его код под этим признаком перестаёт
# быть мёртвым. Поштучный линт ловит то, что не собирается или не
# используется в самом пакете.
set -euo pipefail

case "${1:-}" in
  workspace)
    mkdir -p .build
    # Pipefail сохраняет отказ Clippy при -D warnings.
    cargo clippy --workspace --all-targets --all-features --message-format=json -- -D warnings \
      | clippy-sarif | tee .build/clippy.sarif | sarif-fmt
    ;;
  packages)
    # Подстановка в заголовке `for` скрыла бы отказ cargo metadata.
    mapfile -t packages < <(cargo metadata --no-deps --format-version 1 --locked | jq -r '.packages[].name' | sort)
    wait "$!"
    if [ "${#packages[@]}" -eq 0 ]; then
      echo "::error::cargo metadata не вернул ни одного пакета"
      exit 1
    fi
    failed=()
    for package in "${packages[@]}"; do
      echo "::group::$package"
      cargo clippy -p "$package" --all-targets --locked -- -D warnings || failed+=("$package")
      echo "::endgroup::"
    done
    if [ "${#failed[@]}" -ne 0 ]; then
      echo "::error::Clippy отдельно не проходят: ${failed[*]}"
      exit 1
    fi
    ;;
  *)
    echo "usage: $0 workspace|packages" >&2
    exit 2
    ;;
esac

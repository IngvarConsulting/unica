#!/usr/bin/env bash
# Исследование: снимок исходного дерева платформы 8.3.27 после `cf init`.
#
# Вопрос: какое дерево исходников выписывает платформа 8.3.27 на пустой
# конфигурации, чтобы сверить его с `ibcmd` вручную.
# Метод: публичный `cf init` пишет дерево в пустой каталог
# UNICA_CF_INIT_PLATFORM_EVIDENCE_DIR; путь к нему печатается в конце.
# Входы: установленная платформа 8.3.27; пустой абсолютный каталог вне дерева
# репозитория. Результат — улика вне git; вывод сверяется руками.
set -euo pipefail
cd "$(dirname "$0")/../.."
: "${UNICA_CF_INIT_PLATFORM_EVIDENCE_DIR:?задайте пустой абсолютный каталог для улики}"
cargo test -p unica-coder --features research --test research_cf_init_checkpoint -- --exact public_cf_init_writes_platform_checkpoint_source --nocapture

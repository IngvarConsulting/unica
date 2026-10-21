#!/usr/bin/env bash
# Исследование: каталог событий платформы из её справки.
#
# Вопрос: какие события и у каких владельцев знает платформа по своей справке,
# чтобы оракул событий отвечал по данным, а не по памяти.
# Метод: справка из UNICA_PLATFORM_HELP_DIR разбирается механически, компактный
# снимок пишется в исходники (см. FIXTURE_PATH в platform_help_oracle.rs).
# Входы: каталог справки платформы; UNICA_UPDATE_PLATFORM_EVENT_CATALOG=1 —
# явное согласие на перезапись. Результат закрепляется в дереве; равенство
# со снимком проверяет ignored-оракул того же файла на стенде со справкой
# (`--run-ignored only`, команда в шапке platform_help_oracle.rs).
set -euo pipefail
cd "$(dirname "$0")/../.."
: "${UNICA_PLATFORM_HELP_DIR:?задайте каталог справки платформы}"
UNICA_UPDATE_PLATFORM_EVENT_CATALOG=1 cargo test -p unica-coder --features research --lib -- --ignored regenerate_checked_platform_event_catalog --nocapture

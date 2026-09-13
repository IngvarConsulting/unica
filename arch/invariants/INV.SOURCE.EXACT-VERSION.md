---
id: INV.SOURCE.EXACT-VERSION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/format_guard.rs::version_owning_target_cannot_hide_behind_supported_source_set_owner
scope: [source]
---

# Версия самой цели старше версии окружающего набора

Если изменяемая цель сама несёт неподдерживаемую версию, поддерживаемая версия
окружающего набора исходников её не скрывает: операция отказывает без изменения
байтов цели.

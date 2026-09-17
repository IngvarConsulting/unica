---
id: INV.APP.DIAGNOSTICS-TIMEOUT-IMMUTABLE-OVERLAY
check:
  - crates/unica-coder/src/domain/operational_config.rs::explicit_diagnostics_timeout_is_validated_and_overlaid_immutably
---

# Изменение таймаута не меняет исходные настройки

Если для диагностики явно задано другое время ожидания — таймаут, Unica
создаёт копию настроек с новым значением. Исходные настройки остаются прежними.

Например, при замене 120 секунд на 900 копия получает 900, а в исходных
настройках остаётся 120. Связанный тест проверяет оба значения.

---
id: INV.APP.CONFIG-SNAPSHOT
check:
  - crates/unica-coder/src/domain/operational_config.rs::explicit_diagnostics_timeout_is_validated_and_overlaid_immutably
---

# Явный таймаут диагностики не меняет общую конфигурацию

Переопределение таймаута диагностики создаёт новый снимок настроек.
Исходный снимок сохраняется, поэтому отдельный вызов не меняет таймаут
следующих операций.

Допустимы значения от 30 до 3600 секунд включительно. Выход за эти границы
возвращает `OutOfRange` для явного аргумента `timeoutSeconds`.

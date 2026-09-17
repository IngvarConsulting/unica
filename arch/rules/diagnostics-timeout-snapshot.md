---
id: INV.APP.DIAGNOSTICS-TIMEOUT-IMMUTABLE-OVERLAY
check:
  - crates/unica-coder/src/domain/operational_config.rs::explicit_diagnostics_timeout_is_validated_and_overlaid_immutably
---

# Наложение таймаута сохраняет исходную конфигурацию

Явный таймаут диагностики применяется к новому снимку операционной
конфигурации. Значение в исходном снимке остаётся прежним.

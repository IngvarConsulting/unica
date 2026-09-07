---
id: INV.SOURCE.WRITE-TARGET-KIND
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check:
  - crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs::platform_xml_target_kind_policy_table_is_closed
  - crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs::platform_xml_source_root_handle_revalidates_without_widening
  - crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs::platform_xml_source_target_revalidation_rejects_changed_descriptor_identity
scope: [source]
---

# Писатель принимает только терминал модуля

Разрешение цели выполняется под явной политикой вида. Пишущая операция
запрашивает только терминал модуля и отклоняет любой другой вид стабильным
`TargetKindMismatch`; закрытая ручка несёт выданный вид и перепроверяется под
той же политикой, поэтому расширение резолвера не расширяет право записи.

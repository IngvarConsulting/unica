---
id: INV.SOURCE.WRITE-TARGET-KIND
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs::write_target_kind_and_revalidation_contract_is_complete
scope: [source]
---

# Писатель принимает только терминал модуля

Разрешение цели выполняется под явной политикой вида. Пишущая операция
запрашивает только терминал модуля и отклоняет любой другой вид стабильным
`TargetKindMismatch`; закрытая ручка несёт выданный вид и перепроверяется под
той же политикой, поэтому расширение резолвера не расширяет право записи.

---
id: INV.SOURCE.CODE-PATCH-BATCH-ATOMICITY
status: active
governs: product
decision: DEC.2026-08-22.ATOMIC-CODE-REPLACEMENT-BATCH
check: crates/unica-coder/src/infrastructure/native_operations/code.rs::code_patch_batch_count_mismatch_and_overlap_write_nothing
scope: [source]
---

# Ошибка пакета замен не пишет модуль

Несовпадение `expectedCount` и пересечение диапазонов пакетной замены оставляют
байты BSL-модуля неизменными.

---
id: INV.SOURCE.READER-OUTPUT-PARITY
status: superseded
governs: product
decision: DEC.2026-09-02.V0-13-LEGACY-BATCH-1
check: crates/unica-coder/src/infrastructure/native_operations/source_invariant_tests.rs::bridged_reader_outputs_are_identical_for_logical_and_physical_selectors
scope: [source]
---

# Мост читателя не меняет типизированный ответ

Ровно тринадцать читателей, которые называет
`authoritative_reader_migration_inventory`, отвечают в режиме `bridge` на
логический вызов теми же типизированными данными, что на вызов своим файловым
селектором.

---
id: INV.SOURCE.READER-OUTPUT-PARITY
status: superseded
governs: product
decision: DEC.2026-09-02.V0-13-LEGACY-BATCH-1
check: tests/ci/test_v013_parity_inventory.py::test_each_referenced_case_exists_matches_successor_and_has_one_owner
scope: [source]
---

# Мост читателя не меняет типизированный ответ

Ровно тринадцать читателей, которые называет
`authoritative_reader_migration_inventory`, отвечают в режиме `bridge` на
логический вызов теми же типизированными данными, что на вызов своим файловым
селектором.

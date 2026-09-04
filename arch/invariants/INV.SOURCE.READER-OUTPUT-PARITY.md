---
id: INV.SOURCE.READER-OUTPUT-PARITY
status: superseded
governs: product
decision: DEC.2026-09-04.V0-13-LEGACY-BATCH-3
check: tests/ci/test_acceptance_scenarios.py::test_every_wire_answers_its_frozen_classes
scope: [source]
---

# Мост читателя не меняет типизированный ответ

Ровно тринадцать читателей, которые называет
`authoritative_reader_migration_inventory`, отвечают в режиме `bridge` на
логический вызов теми же типизированными данными, что на вызов своим файловым
селектором.

Мост читателей снят вместе с ними: одинаковость ответа на два селектора
больше не имеет предмета, а корпус приёмки замораживает ответы канонических
чтений на тех же узлах.

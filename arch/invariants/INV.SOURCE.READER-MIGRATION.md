---
id: INV.SOURCE.READER-MIGRATION
status: superseded
governs: product
decision: DEC.2026-09-04.V0-13-LEGACY-BATCH-3
check: tests/ci/test_acceptance_scenarios.py::test_every_wire_answers_its_frozen_classes
scope: [source]
---

# Режим миграции читателя объявлен явно

Единый инвентарь объявляет тринадцать предметных читателей в режиме `bridge` и
единственный `directSwitch` для `unica.code.diagnostics`; каждый мост сохраняет
две взаимоисключающие ветви схемы, а прямой переход не публикует старые поля.

Инвентарь режимов миграции снят вместе с читателями, которые он объявлял:
мостов не осталось, а канонический путь заморожен корпусом приёмки.

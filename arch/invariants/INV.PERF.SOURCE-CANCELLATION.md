---
id: INV.PERF.SOURCE-CANCELLATION
status: superseded
governs: product
decision: DEC.2026-09-05.SOURCE-SNAPSHOT-PROVIDER-RETIRED
check: tests/ci/test_acceptance_scenarios.py::test_every_wire_answers_its_frozen_classes
scope: [product, source]
---

# Отмена проверяется между фазами снимка

Отмена между разрешением цели и публикацией снимка прекращает операцию до
выдачи частичного результата.

Правило снято вместе с поставщиком снимков: у канонического чтения нет фаз
разрешения и публикации, между которыми проверялась отмена.

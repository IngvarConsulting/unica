---
id: INV.SOURCE.WRITABLE-PROFILE-GATE
status: active
governs: product
decision: DEC.2026-09-02.V0-13-LEGACY-BATCH-1
check: tests/ci/test_acceptance_scenarios.py::test_every_wire_answers_its_frozen_classes
scope: [app, source]
---

# Канонический путь держит страж формата выгрузки

`unica.check` с профилем над корнем вне активного профиля ведёт диагностику
предупреждением закрытого кода формата, а цель, которую порт чтения не
открывает, отвечает `invalid_source` с указанием формата. `unica.apply` над узлом,
чья цепочка владельцев содержит корень вне записываемого профиля, отказывает
`invalid_source` до первого байта. Корпус приёмки замораживает оба ответа на
наборах `newer`, `unversioned` и `nosupport` фикстуры.

---
id: INV.SOURCE.SUBSYSTEM-ADDRESS
status: active
governs: product
decision: DEC.2026-09-04.V0-13-LEGACY-BATCH-3
check: tests/ci/test_acceptance_scenarios.py::test_every_wire_answers_its_frozen_classes
scope: [source]
---

# Адрес подсистемы следует диалекту БСП

Чтение подсистемы адресуется логически — `Subsystem.Родитель` — и отвечает
зарегистрированным деревом имён, не публикуя в типизированных данных физические
`Subsystems/`, обратную косую черту или `.xml`.

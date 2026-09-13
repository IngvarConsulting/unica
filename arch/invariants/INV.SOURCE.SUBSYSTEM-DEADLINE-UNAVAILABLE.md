---
id: INV.SOURCE.SUBSYSTEM-DEADLINE-UNAVAILABLE
status: active
governs: product
decision: DEC.2026-09-04.V0-13-LEGACY-BATCH-3
check: tests/ci/test_acceptance_scenarios.py::test_every_wire_answers_its_frozen_classes
scope: [source]
---

# Истечение срока не публикует проекцию подсистемы

Истечение срока во время зарегистрированной предпроверки `subsystem.info`
завершает вызов ошибкой `provider deadline exceeded`, не помечает её как
`provider_unavailable` и возвращает `data: null`, не публикуя частичную или
пустую доказанную проекцию.

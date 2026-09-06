---
id: INV.PERF.SOURCE-SNAPSHOT-TTL
status: superseded
governs: product
decision: DEC.2026-09-05.SOURCE-SNAPSHOT-PROVIDER-RETIRED
check: tests/ci/test_acceptance_scenarios.py::test_every_wire_answers_its_frozen_classes
scope: [product, source]
---

# Страница и чтение истекают на границе срока снимка

После точной границы TTL снимок одинаково недоступен для продолжения страницы и
для чтения ресурса.

Правило снято вместе с хранилищем снимков: курсор канонического `view` связан
с ревизией, а не со сроком жизни снимка.

---
id: INV.PERF.SOURCE-RESOURCE-LIMITS
status: superseded
governs: product
decision: DEC.2026-09-05.SOURCE-SNAPSHOT-PROVIDER-RETIRED
check: tests/ci/test_acceptance_scenarios.py::test_every_wire_answers_its_frozen_classes
scope: [product, source]
---

# Лимиты ресурсов источника являются явным контрактом

Снимок содержит не более 100 ресурсов, страница — не более 50, чтение — не
более 64 КиБ, срок жизни равен пяти минутам, а полнота имеет закрытые значения.

Правило снято вместе с читателями `unica.source.resources` и
`unica.source.read`: канонический `view` отвечает страницей по курсору, а не
манифестом снимка с этими лимитами.

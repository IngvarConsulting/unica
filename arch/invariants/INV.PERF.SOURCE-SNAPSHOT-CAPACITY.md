---
id: INV.PERF.SOURCE-SNAPSHOT-CAPACITY
status: superseded
governs: product
decision: DEC.2026-09-05.SOURCE-SNAPSHOT-PROVIDER-RETIRED
check: tests/ci/test_acceptance_scenarios.py::test_every_wire_answers_its_frozen_classes
scope: [product, source]
---

# Живые снимки ограничены без скрытого вытеснения

Хранилище отклоняет новый снимок при исчерпании вместимости и не вытесняет
неистёкший снимок, на который ещё опирается вызывающая сторона.

Правило снято вместе с хранилищем снимков: канонический `view` не держит
снимков между вызовами, и вытеснять ему нечего.

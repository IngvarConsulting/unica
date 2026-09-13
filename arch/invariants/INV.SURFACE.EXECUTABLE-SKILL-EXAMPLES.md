---
id: INV.SURFACE.EXECUTABLE-SKILL-EXAMPLES
status: superseded
governs: product
decision: DEC.2026-09-02.V0-13-LEGACY-BATCH-1
check: tests/ci/test_acceptance_scenarios.py::test_every_wire_answers_its_frozen_classes
scope: [wire]
---

# Примеры вызовов скиллов исполняются по своему режиму

Каждый JSON-пример `tools/call` из поставляемых скиллов исполняется на
детерминированной фикстуре как чтение либо как предпросмотр мутации.

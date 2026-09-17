---
id: INV.REGISTRY.BINDING-IS-REALIZED
status: active
governs: process
decision: DEC.2026-08-19.REALIZATION-AXIS
check: tests/arch/test_registry.py::test_binding_and_planned_decisions_do_not_claim_the_same_state
scope: [docs]
---

# Действующее решение не бывает непостроенным

Действующее решение называет существующее свидетельство реализации. Ещё не
реализованное направление помечено как запланированное и такого свидетельства
не называет.

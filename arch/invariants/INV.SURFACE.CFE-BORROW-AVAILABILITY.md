---
id: INV.SURFACE.CFE-BORROW-AVAILABILITY
status: active
governs: product
decision: DEC.2026-09-14.CFE-BORROWED-STRUCTURE
check: tests/ci/test_unica_skills.py::test_cfe_borrow_guidance_does_not_claim_a_public_borrower
scope: [docs, wire]
---

# Скилл заимствования называет отсутствие публичного маршрута

Скилл `cfe-borrow` прямо сообщает, что заимствование пока недоступно.
Его исполняемые примеры содержат только `unica.view` с секцией `can` и
`unica.check` корня расширения.

---
id: INV.SURFACE.ROLE-EDIT-LOGICAL
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_skills.py::test_role_edit_skill_uses_only_the_logical_typed_contract
scope: [wire]
---

# Пример role.edit использует логическую типизированную форму

Поставляемый пример `unica.role.edit` выбирает роль через `sourceSet` и
`metadataPath` и передаёт типизированные операции `setRight` без физических
селекторов прежнего интерфейса.

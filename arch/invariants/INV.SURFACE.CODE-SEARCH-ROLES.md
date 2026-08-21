---
id: INV.SURFACE.CODE-SEARCH-ROLES
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_release_assessment.py::test_code_search_is_blocking_and_requires_fixed_role_sections
scope: [wire]
---

# Приёмка поиска требует три ролевые секции

Релизная приёмка отклоняет ответ `unica.code.search`, если в нём нет секций
`semantic`, `symbol` и `lexical`.

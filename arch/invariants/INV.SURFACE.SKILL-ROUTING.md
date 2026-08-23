---
id: INV.SURFACE.SKILL-ROUTING
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_skills.py::test_in_scope_skills_route_to_single_unica_mcp
scope: [wire]
---

# Предметные скиллы маршрутизируются через unica

Каждый предметный скилл из проверяемого набора называет MCP `unica`, свой
инструмент `unica.*` и не направляет модель к внутреннему серверу-адаптеру.

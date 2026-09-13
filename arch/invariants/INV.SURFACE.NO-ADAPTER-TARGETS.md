---
id: INV.SURFACE.NO-ADAPTER-TARGETS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_skills.py::test_all_skills_do_not_expose_internal_mcp_names
scope: [wire]
---

# Скиллы не показывают внутренние MCP-имена

Поставляемые скиллы не содержат идентификаторов внутренних MCP-серверов
ядра, runtime, анализаторов и стандартов.

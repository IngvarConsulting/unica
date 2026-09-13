---
id: INV.SURFACE.RUNTIME-NO-SKILL
status: active
governs: product
decision: DEC.2026-09-02.RUNTIME-IS-TOOL-NATIVE
check: tests/ci/test_unica_skills.py::test_runtime_is_tool_native_and_v8_runner_skill_is_not_shipped
scope: [product, wire]
---

# Runtime не публикуется отдельным skill

Каталог `plugins/unica/skills/v8-runner` отсутствует, а основные runtime-маршруты
не предлагают вызвать skill с этим именем. Внутренний движок не считается
публичным skill и остаётся доступен только через контракт MCP Unica.

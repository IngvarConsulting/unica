---
id: INV.SURFACE.SKILL-PREVIEW-GUIDANCE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_skills.py::test_migrated_skills_use_task_parameterized_mcp_examples
scope: [wire]
---

# Поставляемые meta-сценарии сохраняют явный dry-run пример

Скиллы `meta-add` и `meta-remove` содержат задачно-параметризованные MCP-примеры
с литералом `"dryRun": true`.

---
id: INV.SURFACE.RUNTIME-NO-SKILL
check:
  - tests/ci/test_unica_skills.py::UnicaSkillRoutingTests.test_runtime_is_tool_native_and_v8_runner_skill_is_not_shipped
  - tests/ci/test_package_unica_plugin.py::PackageUnicaPluginTests.test_generated_marketplace_is_thin_pinned_and_target_neutral
---

# Пакет не поставляет v8-runner как отдельный skill

В исходном плагине и собранном marketplace-пакете нет каталога
`skills/v8-runner` с исполняемым сценарием или справочными файлами.
Публичные runtime-операции доступны через
[словарь unica.run](../mcp/run-operation-names.md).

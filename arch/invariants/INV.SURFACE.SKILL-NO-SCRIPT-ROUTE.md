---
id: INV.SURFACE.SKILL-NO-SCRIPT-ROUTE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_skills.py::test_migrated_skills_do_not_reference_skill_local_operation_scripts
scope: [wire]
---

# Мигрированные скиллы не маршрутизируют в локальные скрипты

Проверяемые `SKILL.md` не содержат перечисленные стражем ссылки на `.py`,
`.ps1`, `powershell.exe` и прежние формулировки fallback/native execution.

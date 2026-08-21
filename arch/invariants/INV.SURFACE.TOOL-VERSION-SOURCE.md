---
id: INV.SURFACE.TOOL-VERSION-SOURCE
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_skill_provenance.py::test_tool_lock_ref_uses_tools_lock_as_single_binary_baseline
scope: [pkg]
---

# Происхождение runtime ссылается на lock-файл

Запись происхождения `v8-runner` выбирает бинарную версию через `toolLockRef`,
который разрешается в `tools.lock.json`, а не повторяет версию отдельно.

---
id: INV.SURFACE.TOOL-VERSION-SOURCE
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_skill_provenance.py::test_v8_runner_tool_lock_ref_resolves_to_locked_baseline
scope: [pkg]
---

# Ссылка происхождения v8-runner разрешается в lock-файл

Запись происхождения `v8-runner-rust` содержит `toolLockRef = v8-runner`, а
соответствующая запись `tools.lock.json` закрепляет ожидаемые `sourceTag` и
`sourceCommit`.

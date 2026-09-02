---
id: DEC.2026-09-02.RUNTIME-IS-TOOL-NATIVE
status: active
governs: product
realized: tests/ci/test_unica_skills.py::test_runtime_is_tool_native_and_v8_runner_skill_is_not_shipped
supersedes: []
superseded-by: null
establishes: [INV.SURFACE.RUNTIME-NO-SKILL]
design: docs/design/2026-09-02-runtime-is-tool-native-design.md
---

# Runtime является контрактом инструмента, а не отдельным skill

**Решение.** Пакет не поставляет skill `v8-runner` и его локальные references.
Runtime-намерения обнаруживаются через `unica.run {}`; модель использует только
операцию с `implemented: true` и опубликованный для неё `argsSchema`. Движок
`v8-runner`, его provenance и поставка остаются внутренней реализацией Unica.
Предметные skills не направляют модель к удалённому runtime skill.

---
id: DEC.2026-08-31.V0-13-SURFACE-FIRST-CUTOVER
status: active
governs: product
realized: crates/unica-coder/src/interfaces/mcp.rs::production_mcp_surface_exposes_only_canonical_v13_tools_and_task_compatibility
supersedes: [DEC.2026-08-23.V0-13-EXECUTION-SURFACE]
superseded-by: null
establishes: [CTR.WIRE.TOOL-SURFACE, INV.WIRE.SURFACE-RELEASE-ROUTING, INV.PKG.PACKAGED-PUBLIC-SURFACE, INV.APP.V13-USEFUL-PARTIAL-MODES]
changes: [CTR.WIRE.TOOL-SURFACE]
design: docs/design/2026-08-31-v0-13-surface-first-cutover-design.md
---

# v0.13 переключает имена раньше полной предметной паритетности

**Решение.** Package-selected MCP публикует ровно восемь предметных инструментов
`unica.view`, `unica.apply`, `unica.find`, `unica.search`, `unica.check`,
`unica.diff`, `unica.run`, `unica.docs` клиенту с native Tasks и добавляет только
`unica.task.get`, `unica.task.result`, `unica.task.cancel` клиенту без них.
Имена v0.12 удаляются атомарно; alias и смешанный профиль запрещены.

Cutover является surface-first: каждый предметный инструмент имеет хотя бы один
полезный замкнутый режим, но вся семантика 74 опубликованных вызовов v0.12.3 не
объявляется перенесённой. Неперенесённые режимы отвечают типизированным
`unsupported_*`, а не ложным `provider_unavailable` или успехом без эффекта.
Точная таблица перехода принадлежит design-документу и проверяется против
неизменяемого v0.12.3 baseline.

`plugins/unica/skills/**` не меняются этим cutover и проверяются владельцем
отдельно. Попадание изменений в `main` не означает публикацию релиза: package и
release gates остаются самостоятельными.

**Почему.** Цена полной предметной миграции удерживала новую поверхность за
старыми именами, хотя daemon, Invocation и Task transport уже позволяют отделить
стабильный внешний словарь от поэтапного наполнения режимов.

**Цена.** После переключения часть прежних сценариев временно получает честный
typed unsupported; паритет переносится по строкам матрицы, а не возвратом старых
публичных инструментов.

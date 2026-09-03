---
id: DEC.2026-09-02.DIRECTIONAL-RUNTIME-OPERATIONS
status: superseded
governs: product
realized: crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_dictionary_has_twelve_directional_runtime_intents
supersedes: [DEC.2026-08-31.V0-13-FIRST-IMPLEMENTATION-VERTICALS, DEC.2026-08-31.V0-13-NO-QUERY-EXECUTE, DEC.2026-09-02.RUN-INITIALIZATION-CONTRACT]
superseded-by: DEC.2026-09-03.INFOBASE-EXPORT-RUN-SLICE
establishes: [CTR.WIRE.TOOL-SURFACE, INV.APP.V13-IMPLEMENTATION-COVERAGE, INV.APP.V13-RUN-DICTIONARY, INV.SURFACE.ARGUMENTS-DESCRIBED, INV.SURFACE.PROJECT-READINESS, INV.SURFACE.SOURCE-ATTACH, INV.SURFACE.WORKSPACE-BOOTSTRAP, INV.SURFACE.WORKSPACE-INITIALIZE, INV.SURFACE.RUN-INTENTS-DIRECTIONAL]
changes: [CTR.WIRE.TOOL-SURFACE]
design: docs/design/2026-09-02-directional-runtime-operations-design.md
---

# Run называет направление данных и не смешивает разные runtime-состояния

**Решение.** `unica.run {}` остаётся одним discovery/execute-инструментом и
публикует данными двенадцать намерений. Имена отличают source build от экспорта
состояния ИБ, CF/CFE от DT и полный snapshot от конфигурации. `syntax.check`,
`test.run` и неоднозначный `extension.sync` в v0.13 не публикуются. Инициализация
workspace называется `workspace.initialize`; реализованный source-only срез
сохраняет прежний revision-fenced create-only контракт.

Отдельный машиночитаемый реестр реализованности сохраняется: форма typed result
не доказывает наличие движка. В этом срезе поддержан только
`workspace.initialize`; остальные одиннадцать намерений честно помечены
`unsupported`. `query.execute` также остаётся удалённым из v0.13.

Планируемые операции видимы в словаре с `implemented=false`, но `view.next`
содержит только исполнимое продолжение. Изменяющие и создающие файлы намерения
всегда проходят неисполняющий preview и fenced apply. Выбор platform provider,
таймаут, fallback и terminal receipt принадлежат runner, а не модели и не
аргументам публичного MCP.

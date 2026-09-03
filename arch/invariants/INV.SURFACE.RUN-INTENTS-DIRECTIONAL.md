---
id: INV.SURFACE.RUN-INTENTS-DIRECTIONAL
status: active
governs: product
decision: DEC.2026-09-03.INFOBASE-EXPORT-RUN-SLICE
check: crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_dictionary_has_twelve_directional_runtime_intents
scope: [wire]
---

# Runtime-намерения называют источник и назначение состояния

Словарь `unica.run {}` отдельно называет source build/dump, artifact build,
CF/CFE configuration export/load и DT infobase dump/restore. Он не публикует
`syntax.check`, `test.run`, `extension.sync` и generic artifact load/make.
Реализованная операция публикует закрытую `argsSchema`; нереализованная не
выдаёт предположение о ещё не принятом контракте аргументов.

`infobase.configuration.export` принимает `state`, workspace-relative
`output` и необязательное имя `extension`; `infobase.dump` принимает только
workspace-relative `output`. Обе схемы закрыты и не передают модели выбор
provider.

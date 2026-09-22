---
id: INV.SURFACE.RUN-INTENTS-DIRECTIONAL
status: active
governs: product
decision: DEC.2026-09-22.RUNNER-ONE-TARGET-VOCABULARY
check: crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_dictionary_has_twelve_directional_runtime_intents
scope: [wire]
---

# Runtime-намерения различают источник и назначение состояния

Словарь сохраняет различие исходников (`push`/`pull`), пакетов
(`upload`/`download`, `make`) и базы целиком (`infobase.dump`/`infobase.restore`).
Имена и доступность определены INV.WIRE.RUNNER-ONE-VOCABULARY и
INV.RUNTIME.RUNNER-ONE-CAPABILITIES. Схемы не дают выбирать платформенного
провайдера. `download` требует состояние и путь назначения; `infobase.dump`
требует путь DT. Типизированный аргумент не является строкой CLI.

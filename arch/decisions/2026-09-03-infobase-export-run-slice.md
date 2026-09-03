---
id: DEC.2026-09-03.INFOBASE-EXPORT-RUN-SLICE
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/daemon/server.rs::v5_infobase_exports_prepare_before_source_admission_and_keep_the_revision_gate
supersedes: [DEC.2026-09-02.DIRECTIONAL-RUNTIME-OPERATIONS]
superseded-by: null
establishes: [CTR.WIRE.TOOL-SURFACE, INV.APP.V13-IMPLEMENTATION-COVERAGE, INV.APP.V13-RUN-DICTIONARY, INV.RUNTIME.V13-INFOBASE-EXPORTS, INV.SURFACE.ARGUMENTS-DESCRIBED, INV.SURFACE.PROJECT-READINESS, INV.SURFACE.RUN-INTENTS-DIRECTIONAL, INV.SURFACE.WORKSPACE-BOOTSTRAP, INV.SURFACE.WORKSPACE-INITIALIZE]
changes: [CTR.WIRE.TOOL-SURFACE]
design: docs/design/2026-09-03-infobase-export-run-slice-design.md
---

# Первые runtime-выгрузки v0.13 принадлежат Run и v8-runner

**Решение.** `infobase.configuration.export` и `infobase.dump` становятся
первыми исполняемыми runtime-намерениями после `workspace.initialize`. MCP
принимает только предметные параметры; provider, fallback и платформенный
таймаут выбирает сопровождаемый `v8-runner` версии 0.7.1.

Обе операции являются долгими Task. Preview обязательно запускает тот же
provider resolver через `--dry-run`, но не provider. Revision связывает точные
аргументы, основной и локальный config, существующий output, версию runner и
его выбранный план. Apply повторяет preview и при совпавшем `ifRev` запускает
экспорт. Unica не доверяет одному success envelope и отдельно подтверждает
regular непустой файл и его SHA-256.

Workspace с `infobase.connection` и без source set больше не считается
неинициализированным: `unica.view {}` показывает это состояние без ложной
диагностики и предлагает только preview CF и DT. Загрузка в ИБ, создание ИБ и
выгрузка исходников остаются `unsupported` до отдельных вертикалей.

Обе внутренние runtime-версии маршрутизируют эти две операции в общий
`PreparedInfobaseExport` до admission PlatformXml workspace actor. Поэтому
выбор V3 или V5 transport не меняет доступность выгрузок для infobase-only
workspace и не ослабляет revision gate.

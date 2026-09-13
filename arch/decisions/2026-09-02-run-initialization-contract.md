---
id: DEC.2026-09-02.RUN-INITIALIZATION-CONTRACT
status: superseded
governs: product
realized: crates/unica-coder/tests/v13_workspace_bootstrap.rs::canonical_stdio_previews_and_applies_autodetected_source_attachment_before_admission
supersedes: [DEC.2026-09-01.VIEW-WORKSPACE-BOOTSTRAP]
superseded-by: DEC.2026-09-02.DIRECTIONAL-RUNTIME-OPERATIONS
establishes: [CTR.WIRE.TOOL-SURFACE, INV.SURFACE.ARGUMENTS-DESCRIBED, INV.SURFACE.PROJECT-READINESS, INV.SURFACE.WORKSPACE-BOOTSTRAP, INV.SURFACE.SOURCE-ATTACH]
changes: [CTR.WIRE.TOOL-SURFACE]
design: docs/design/2026-09-02-run-initialization-contract-design.md
---

# Инициализация workspace является preview/apply-намерением Run

**Решение.** Первый неизвестный клиент начинает с read-only `unica.view {}`.
Пустой workspace получает одну первичную диагностику без нерелевантного health,
а autodetected однородные source sets получают исполнимое продолжение
`unica.run` `source.attach`. Изменяющее намерение требует явного `dryRun`:
preview не меняет окружение и возвращает revision-fenced apply-вызов; apply
create-only публикует `v8project.yaml` и не требует платформы 1С.

Закрытый словарь двенадцати намерений сохраняется и доступен до source
admission. Он сообщает модели требования preview/fence. Смешанные EDT/Designer
наборы не сворачиваются в формат effective source set, существующий config не
перезаписывается, а не реализованные намерения не выдаются в `next`.

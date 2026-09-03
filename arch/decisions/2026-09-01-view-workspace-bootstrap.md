---
id: DEC.2026-09-01.VIEW-WORKSPACE-BOOTSTRAP
status: superseded
governs: product
realized: crates/unica-coder/tests/v13_workspace_bootstrap.rs::canonical_stdio_bootstraps_an_empty_workspace_before_address_discovery
supersedes: []
superseded-by: DEC.2026-09-02.RUN-INITIALIZATION-CONTRACT
establishes: [CTR.WIRE.TOOL-SURFACE, INV.SURFACE.ARGUMENTS-DESCRIBED, INV.SURFACE.PROJECT-READINESS, INV.SURFACE.WORKSPACE-BOOTSTRAP]
changes: [CTR.WIRE.TOOL-SURFACE]
design: docs/design/2026-09-01-view-workspace-bootstrap-design.md
---

# Первый вызов `view` открывает рабочее пространство

**Решение.** `unica.view {}` является read-only bootstrap-вызовом до admission
source set и возвращает рабочий корень, состояние `v8project.yaml`, найденные
source sets, раздельную готовность исходников и репозитория, безопасный рецепт
настройки там, где один файл может точно выразить discovery, и только исполнимое
для ready Platform XML продолжение с канонически кодируемым source-set именем.
`unica.view {at}` сохраняет actor-owned
чтение логического узла; параметры проекции без `at` запрещены.

Восемь канонических tools и все их аргументы несут краткое описание, а
`initialize.instructions` направляет неизвестного клиента сначала в
`unica.view {}`. Поставляемые skills/references не направляют к снятым project
tools. Не реализованные mutation-режимы не выдаются за продолжение.

**Почему.** Обязательный `at` создавал цикл: source set нужен для адреса, но
публичного дискавери source set после cutover не осталось.

**Цена.** У `view` появляется второй режим, а daemon обязан обслужить одно
ограниченное handoff-бюджетом наблюдение до создания actor identity.

# 1. Введение и цели

## Цель

Unica предоставляет Codex-плагин для повседневной разработки 1C:Enterprise:
инициализация workspace, работа с XML-исходниками конфигурации, формами, СКД,
MXL, ролями, сборкой, диагностикой, справочной информацией и полным безопасным
циклом задач для конфигураций, связанных с хранилищем 1С.

## Главная архитектурная цель

Для LLM должен существовать один публичный MCP server: `unica`. Все остальные
движки являются внутренними adapters, чтобы синхронизация кешей, индексов и
workspace state происходила внутри orchestrator, а не через модель.

## Stakeholders

- AI agent: вызывает стабильные tools `unica.*` и получает компактный structured
  result.
- 1C developer: получает operation skills and MCP tools without needing to run
  skill-local operation files directly.
- Maintainer: обновляет bundled tools, skills, Rust orchestrator и specs без
  нарушения public MCP contract.

## Goals

1. Один публичный MCP contract.
2. Минимальный расход контекста LLM на инфраструктурную координацию.
3. Явное владение cache/state внутри Rust orchestrator.
4. Native Rust MCP handlers own command semantics for operation backends.
5. Проверяемый packaging и fresh Codex visibility.
6. Воспроизводимый branched-development от чистого distribution baseline до
   одного финального task-content repository commit, verified unlock, archive,
   recovery и owned cleanup; отдельная root-only support prerequisite chain
   допускается только как типизированное проверенное внешнее условие с
   cancellation/recovery/inverse-cleanup exits.
7. Безопасная конкуренция с хранилищем: глобальный history cursor не подменяет
   relevant baseline, каждый диапазон версий классифицируется, selective
   `-Objects` update проверяется по объектам, а root guard не изображает
   блокировку unrelated commits.
8. Любая автоматизация вокруг ручной поддержки остаётся fail-closed: сначала
   человек только захватывает корень, затем отдельный
   строго read-only preview `supportPrerequisiteArm` без `operationId`,
   `dryRun` и durable preview handle проверяет неизменность истории/поддержки/
   original/handoff; после потери ответа он повторяется. Только отдельный
   `localJournaled` apply с `approvedArmingDigest` публикует durable arming
   receipt и разрешает правку. Инструкция просит удерживать корень, но приёмка
   доказывается первой root/support версией после arming cursor, точными actor/
   IB/delta и отсутствием промежуточной root/support версии; release/reacquire
   без такой версии допустим. Stale на preview или final recheck apply оставляет
   `awaitingArm`, не переводит в `armed` и не отменяет авторизацию; после release
   требуется отдельный `supportPrerequisiteCancellation` с полным proof и
   receipt. Ручное окно и original, и отдельной рабочей ИБ
   терминализируется только после отдельной root-only версии, освобождения
   корня и закрытия Designer под exact exclusive service lease; история
   остаётся привязанной к cursor авторизации, а recovery имеет точные цели и
   один из трёх disposition. Recovery CF удерживается capability-proven
   retention provider, final commit требует доказанного atomic no-force safety
   boundary.

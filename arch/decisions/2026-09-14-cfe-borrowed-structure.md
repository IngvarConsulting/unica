---
id: DEC.2026-09-14.CFE-BORROWED-STRUCTURE
status: active
governs: product
realized: crates/unica-coder/tests/v13_workspace_bootstrap.rs::canonical_stdio_checks_borrowed_cfe_structure_and_reports_borrowing_unavailable
supersedes: []
superseded-by: null
establishes: [CTR.FORMAT.CFE-BORROWED-STRUCTURE, INV.SOURCE.CFE-REBORROW-MODULE-STATES, INV.SURFACE.CFE-BORROW-AVAILABILITY]
changes: [CTR.FORMAT.CFE-BORROWED-STRUCTURE]
---

# Структура заимствованного объекта проверяется через актуальную поверхность

**Решение.** Генератор заимствованных дескрипторов и типизированный metadata
валидатор используют общий физический профиль обязательного `ChildObjects`
для 8.3.27. Прямые роли модулей берутся из `PlatformProfile`; адаптеры
модулей владельца и общей команды сверяются с тем же профилем.

`unica.check` корня набора типа `EXTENSION` проверяет структуру объектов
через CFE-валидатор согласно `INV.SURFACE.CHECK-INFERS-VALIDATORS`.
Внутренний генератор сохраняет состояния подключённых модулей при повторном
заимствовании. Его тесты не доказывают наличие публичной операции.

Публичного маршрута заимствования сейчас нет: `unica.cfe.borrow` отсутствует
в `tools/list`, словарь `apply` не содержит заимствования. Скилл показывает
чтение `can`, проверку уже выгруженного расширения и это ограничение.
Восстановление прежнего инструмента или новый контракт заимствования в эту
работу не входит.

Проверка stdio воспроизводит на обработчике из upstream/main ложный `passed`
для отчёта без `ChildObjects`, а с исправлением получает `failed` и диагностику
`cfe`. Проверка платформенной загрузки CFE здесь не заявляется.

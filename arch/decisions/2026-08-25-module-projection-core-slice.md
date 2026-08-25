---
id: DEC.2026-08-25.MODULE-PROJECTION-CORE-SLICE
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/bsl_module_projection.rs::module_projection_core_contract_is_complete
supersedes: []
superseded-by: null
establishes: [CTR.SOURCE.MODULE-PROJECTION-SHAPE, INV.PLATFORM.MODULE-EVENT-CATALOG]
design: docs/design/2026-08-23-v0-13-module-contract-design.md
---

# Скрытое ядро v0.13 проецирует модули и обработчики

**Решение.** Внутренний профиль v0.13 представляет допустимый модуль сводкой
и шестью отдельными проекциями `Method`, `Region`, `Interface`, `Event`,
`Compilation`, `Body`. Сводка не несёт исходник или список методов, её
счётчики совпадают с фактическими проекциями, а отсутствие физического файла
не отменяет возможные события профиля.

BSL-факты строятся из общего AST vendored parser: сигнатура, документация,
директива, накопленный препроцессор, эффективные контексты, область и
аннотация расширения не выводятся регулярными выражениями. XML `callType`
остаётся фактом привязки. Возможные события и применимость владельцу задаёт
один проверенный каталог для одобренного логического профиля 8.3.27. Применимость
событий формы использует тот же типизированный контекст, что и проверка XML:
regular/extension, вид и имя главного реквизита, родительский metadata owner и
привязка таблицы к dynamic-list source. `implemented` требует одновременно
точной BSL-декларации и допустимого для этого контекста XML `callType`.

Замороженная производная содержит фактически выведенную версию установки,
digest русского и английского Syntax Assistant 8.3.27.2074 и page ID. Полный
corpus из 693 markup-страниц (`689` event leaves и `4` structural index pages)
разбит без остатка и пересечений на события одобренного профиля и именованные
закрытые классы исключений; неизвестный owner классом исключения не становится.
Обработчики HTTP, SOAP и IntegrationService остаются на точных декларативных
владельцах и не дублируются синтетическими событиями.

Профиль явно исключает structural/generic templates без platform event ID,
ordinary-form и неподдержанные managed-form owner kinds, non-module owners и
`ExternalDataSource`: последний отсутствует в адресном профиле Task 12 и
требует отдельного архитектурного расширения, а не неявного нового owner kind.

Широкое `DEC.2026-08-23.MODULE-CONTRACT` остаётся `planned`: этот срез не
публикует `unica.view`, `unica.find`, `unica.apply` или мутацию
`event.implement`. Production-профиль v0.12 и его сериализация не меняются до
единого переключения Task 22.

**Почему.** Последующим читателям и писателю нужен один типизированный модульный
контракт до публичного cutover, иначе они независимо восстановят роли,
контексты и события.

**Цена.** До Task 22 ядро остаётся crate-private и проверяется через прямые
contract tests, а не через публичную MCP-поверхность.

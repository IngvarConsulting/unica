---
id: INV.APP.EVENT-SOURCE
check:
  - crates/unica-coder/src/infrastructure/metadata_operations.rs::meta_add_event_subscription_source_replace_needs_no_catalog_and_round_trips
  - crates/unica-metadata/tests/request_contract.rs::source_parser_accepts_the_closed_nonempty_logical_event_source_algebra
---

# Источник подписки задаётся логическим типом

Типизированное описание источника различает объект, менеджер, набор записей,
определяемый тип и семейство объектов. Например, `catalogObject` означает
объекты справочников и не требует наличия отдельного справочника.

Для менеджера константы явно выбирается `constantManager` или
`constantValueManager`: один адрес даёт два разных класса с разными событиями.
Для остальных менеджеров дополнительный выбор класса запрещён.

Типизированная запись сохраняет выбор в XML, а чтение возвращает тот же
источник. Проверки относятся к внутреннему типизированному контракту;
они не возвращают прежний публичный инструмент `unica.meta.edit`.
[Локальная правка состава](event-subscription-source-local-change.md)
сохраняет логический выбор при изменении XML.

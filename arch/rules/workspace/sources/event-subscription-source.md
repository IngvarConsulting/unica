---
id: INV.APP.EVENT-SOURCE
check:
  - crates/unica-coder/src/infrastructure/daemon/metadata_can_tests.rs::canonical_event_source_family_and_constant_manager_choice_round_trip
  - crates/unica-metadata/tests/request_contract.rs::source_parser_accepts_the_closed_nonempty_logical_event_source_algebra
  - crates/unica-coder/src/infrastructure/daemon/metadata_can_tests.rs::canonical_check_requires_an_event_source_registration_and_descriptor_in_the_extension
  - crates/unica-coder/src/infrastructure/daemon/metadata_can_tests.rs::canonical_check_requires_an_own_named_source_through_nested_defined_types
---

# Источник подписки задаётся логическим типом

Типизированное описание источника различает объект, менеджер, набор записей,
определяемый тип и семейство объектов. Например, `catalogObject` означает
объекты справочников и не требует наличия отдельного справочника.

Для менеджера константы явно выбирается `constantManager` или
`constantValueManager`: один адрес даёт два разных класса с разными событиями.
Для остальных менеджеров дополнительный выбор класса запрещён.

Для именованного источника `unica.check` требует регистрацию в `Configuration.xml`
и дескриптор того же объекта в наборе исходников подписки. В расширении
источник может быть собственным или заимствованным. Определяемый тип проверяется
вместе с доступностью его именованных членов на любой глубине.

Типизированная запись сохраняет выбор в XML, а чтение возвращает тот же
источник. Проверки относятся к внутреннему типизированному контракту;
они не возвращают прежний публичный инструмент `unica.meta.edit`.
[Локальная правка состава](event-subscription-source-local-change.md)
сохраняет логический выбор при изменении XML.

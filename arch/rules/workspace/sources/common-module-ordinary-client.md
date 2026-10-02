---
id: INV.WIRE.COMMON-MODULE-ORDINARY-CLIENT
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_apply_view_round_trip_common_module_ordinary_client
  - crates/unica-coder/src/domain/metadata/info_properties.rs::ordinary_client_read_property_stays_independent_from_the_writer_registry
  - crates/unica-coder/src/application/metadata.rs::add_and_edit_report_the_same_supported_common_module_properties
---

# Общий модуль сохраняет флаг обычного клиента

`props.set` в `unica.apply` записывает логическое свойство
`ClientOrdinaryApplication` только для `CommonModule`. `unica.view` читает
сохранённые `true` и `false` в `props.commonModule.clientOrdinaryApplication`.
Предпросмотр сохраняет файлы и наблюдаемую ревизию. Неверный тип значения
и попытка записать свойство документа отклоняются с `bad_value` при
предпросмотре и применении, сохраняя файлы и ревизию.

Профиль чтения этого свойства остаётся независимым от реестра записи.
При типизированном разборе неизвестного имени свойства общего модуля
диагностика указывает поле и перечисляет допустимые для `CommonModule`
имена из реестра записи. Проверка этой диагностики относится к внутреннему
разбору метаданных; точное поле публичного `apply` она не подтверждает.

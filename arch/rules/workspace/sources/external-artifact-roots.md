---
id: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::retained_external_inventory_is_cancellable_and_has_an_aggregate_byte_bound
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::real_external_sources_are_traversable_without_configuration_xml_and_hide_root_runtime_modules
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::external_inventory_skips_runtime_sidecar_and_fails_closed_on_malformed_or_ambiguous_owner
---

# Один набор внешних обработок или отчётов может содержать несколько объектов

Набор внешних обработок или отчётов перечисляет отдельные артефакты.
Каждый получает собственный логический адрес, например
`artifact_processor:ExternalDataProcessor.Import`. Наличие других артефактов
в том же наборе не заставляет выбирать один из них по умолчанию.

Для такого набора не требуется `Configuration.xml`; его корень не объявляет
модули исполнения конфигурации. Повреждённое или неоднозначное описание
артефакта вызывает явный отказ чтения состава.

Чтение состава внешнего набора ограничено общим объёмом описателей
и реагирует на отмену между порциями работы. Превышение лимита или отмена
не превращаются в успешный неполный перечень.

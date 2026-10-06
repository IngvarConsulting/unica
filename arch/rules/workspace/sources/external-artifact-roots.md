---
id: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::retained_external_inventory_is_cancellable
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::retained_external_inventory_crosses_former_aggregate_bytes_without_losing_owners
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::retained_external_inventory_reads_malformed_tail_after_former_aggregate_bytes
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::retained_external_inventory_cancels_inside_a_large_descriptor_without_publishing_prefix
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

Описатели читаются и разбираются последовательно. Уже прочитанный объём
не уменьшает допустимость следующего описателя: сумма байтов не ограничивает
перечень. Отмена проверяется между порциями чтения, в том числе внутри одного
большого описателя. Повреждённый поздний описатель и отмена не превращаются
в успешный неполный перечень.

Читатель сохраняет один текущий описатель и все имена владельцев; расход
памяти растёт с размером описателя и числом артефактов. Прежние ограничения
одного файла и числа непосредственных элементов каталога пока исправляются
отдельно в https://github.com/IngvarConsulting/unica/issues/1119.

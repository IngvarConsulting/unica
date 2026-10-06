---
id: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::retained_external_inventory_is_cancellable
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::retained_external_inventory_crosses_former_aggregate_bytes_without_losing_owners
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::retained_external_inventory_reads_malformed_tail_after_former_aggregate_bytes
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::retained_external_inventory_cancels_inside_a_large_descriptor_without_publishing_prefix
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::external_inventory_accepts_a_single_owner_descriptor_beyond_eight_mib
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::named_external_owner_view_has_no_hidden_eight_mib_descriptor_fallback
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::external_inventory_retains_all_257_immediate_owners_and_the_last_named_view
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::external_inventory_does_not_limit_noise_before_a_valid_owner
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::external_inventory_rejects_malformed_and_non_utf8_late_tails_without_partial_rows
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::named_external_descriptor_and_owner_evidence_preserve_chunk_cancellation
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::named_external_descriptor_distinguishes_missing_from_unreadable_source
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::named_external_missing_descriptor_preserves_explicit_cancellation
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::external_leaf_preserves_an_explicit_finite_relative_read_limit
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
перечень. Размер одного описателя и число непосредственных элементов каталога
тоже не ограничивают чтение состава и чтение именованного артефакта. Отмена проверяется между порциями чтения, в том числе внутри одного
большого описателя. Повреждённый поздний описатель и отмена не превращаются
в успешный неполный перечень.

Отсутствующий описатель именованного артефакта возвращает `not_found`;
ошибка чтения существующего пути не маскируется под отсутствие. Явная
отмена сохраняется и при отсутствии файла. Явно заданный предел отдельного
чтения продолжает действовать.

Читатель сохраняет один текущий описатель и все имена владельцев; расход
памяти растёт с размером описателя и числом артефактов.

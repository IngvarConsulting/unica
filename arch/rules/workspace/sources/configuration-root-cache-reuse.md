---
id: INV.SOURCE.CONFIGURATION-ROOT-CACHE-REUSE
check:
  - crates/unica-coder/src/infrastructure/v13_large_configuration.rs::retained_registrations_are_reused_until_their_file_changes
  - crates/unica-coder/src/infrastructure/v13_large_configuration.rs::a_stamp_inside_the_racy_window_is_never_retained
  - crates/unica-coder/src/infrastructure/v13_large_configuration.rs::stamps_differ_by_every_identity_field
  - crates/unica-coder/src/infrastructure/v13_large_configuration.rs::a_file_changed_during_its_parse_is_not_retained
  - crates/unica-coder/src/infrastructure/v13_large_configuration.rs::every_payload_file_invalidates_the_retained_root
  - crates/unica-coder/src/infrastructure/v13_large_configuration.rs::a_poisoned_slot_is_emptied_and_reused
  - crates/unica-coder/src/infrastructure/v13_large_configuration.rs::a_poisoned_retained_index_is_rebuilt
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::replaced_support_marker_never_reaches_a_retained_root_with_the_old_state
  - crates/unica-coder/src/infrastructure/daemon/invocation_service.rs::logical_read_retains_root_artifacts_across_calls_and_isolates_sources
---

# Повторное чтение конфигурации использует только неизменившиеся артефакты

Индекс регистраций, состояние поддержки (`Ext/ParentConfigurations.bin`)
и разобранный корень для `Configuration.xml` до 8 МиБ актор хранит для
каждого набора между вызовами. Корень большего файла разбирается заново
при каждом вызове; хранится только его индекс регистраций.

Запись годна, пока у её файлов прежние физическая идентичность, длина,
время изменения содержимого и время изменения метаданных. Запись не
сохраняется, если метка времени неизвестна, лежит в будущем или моложе
двух секунд: такой файл разбирается при каждом вызове. Изменение во время
разбора также запрещает удерживать результат. Изменённый файл разбирается
заново, и устаревшие регистрации или состояние поддержки не выдаются.
Любой файл, от которого зависит представление корня, инвалидирует его
удержанный результат при изменении.

Проверка не обходит дерево, наборы исходников не делят удержанные записи.
Содержимое файлов не хешируется.

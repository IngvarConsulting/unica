---
id: INV.SOURCE.LARGE-CONFIGURATION-ROOT-VIEW
check:
  - crates/unica-coder/tests/v13_search_integration.rs::canonical_view_reads_configuration_past_eight_mebibytes_with_many_registrations
  - crates/unica-coder/src/infrastructure/v13_large_configuration.rs::retained_registrations_are_reused_until_their_file_changes
  - crates/unica-coder/src/infrastructure/v13_large_configuration.rs::a_stamp_inside_the_racy_window_is_never_retained
  - crates/unica-coder/src/infrastructure/v13_large_configuration.rs::stamps_differ_by_every_identity_field
  - crates/unica-coder/src/infrastructure/v13_large_configuration.rs::a_file_changed_during_its_parse_is_not_retained
  - crates/unica-coder/src/infrastructure/v13_large_configuration.rs::every_payload_file_invalidates_the_retained_root
  - crates/unica-coder/src/infrastructure/v13_large_configuration.rs::a_poisoned_slot_is_emptied_and_reused
  - crates/unica-coder/src/infrastructure/v13_large_configuration.rs::a_poisoned_retained_index_is_rebuilt
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::replaced_support_marker_never_reaches_a_retained_root_with_the_old_state
  - crates/unica-coder/src/infrastructure/daemon/invocation_service.rs::logical_read_retains_root_artifacts_across_calls_and_isolates_sources
gap: https://github.com/IngvarConsulting/unica/issues/1119
---

# Просмотр корня конфигурации не отказывает из-за общего размера XML

`view` читает `Configuration.xml` через удержанный файл и проверяет его до
выдачи полного результата. Общий размер этого файла сам по себе не является
причиной отказа при просмотре корня, ветки метаданных или зарегистрированного
объекта. Запросы к объектам используют общий индекс регистраций; неподтверждённая
регистрация не становится успешным чтением.

Индекс регистраций, состояние поддержки (`Ext/ParentConfigurations.bin`)
и разобранный корень для `Configuration.xml` до 8 МиБ актор хранит для
каждого набора между вызовами. Корень большего файла разбирается заново
при каждом вызове; хранится только его индекс регистраций. Запись годна,
пока у её файлов прежние физическая идентичность, длина, время изменения
содержимого и время изменения метаданных. Запись не сохраняется, если
метка времени неизвестна, лежит в будущем или моложе двух секунд: такой
файл разбирается при каждом вызове. Изменённый файл разбирается заново,
и устаревшие регистрации не выдаются. Проверка не обходит дерево, наборы
записей не делят. Содержимое файлов не хешируется.

Полная работа с большой конфигурацией ещё не достигнута: отдельный большой
XML-токен требует памяти пропорционально его длине, чтение дескриптора объекта
по-прежнему ограничено 8 МиБ, а ветка с большим числом уникальных объектов
может превысить квоту снимка коллекции. Эти случаи остаются в [#1119](https://github.com/IngvarConsulting/unica/issues/1119).

---
id: INV.SOURCE.LARGE-CONFIGURATION-ROOT-VIEW
check:
  - crates/unica-coder/tests/v13_search_integration.rs::canonical_view_reads_configuration_past_eight_mebibytes_with_many_registrations
  - crates/unica-coder/src/infrastructure/v13_large_configuration.rs::retained_root_artifacts_are_reused_until_their_file_changes
  - crates/unica-coder/src/infrastructure/daemon/invocation_service.rs::logical_read_retains_root_artifacts_across_calls_and_isolates_sources
gap: https://github.com/IngvarConsulting/unica/issues/1119
---

# Просмотр корня конфигурации не отказывает из-за общего размера XML

`view` читает `Configuration.xml` через удержанный файл и проверяет его до
выдачи полного результата. Общий размер этого файла сам по себе не является
причиной отказа при просмотре корня, ветки метаданных или зарегистрированного
объекта. Запросы к объектам используют общий индекс регистраций; неподтверждённая
регистрация не становится успешным чтением.

Индекс регистраций, разобранный корень и состояние поддержки
(`Ext/ParentConfigurations.bin`) актор хранит для каждого набора между
вызовами. Запись годна, пока у её файлов прежняя физическая идентичность,
длина и время изменения; изменённый файл разбирается заново, и устаревшие
регистрации не выдаются. Проверка не обходит дерево, наборы записей не делят.

Полная работа с большой конфигурацией ещё не достигнута: отдельный большой
XML-токен требует памяти пропорционально его длине, чтение дескриптора объекта
по-прежнему ограничено 8 МиБ, а ветка с большим числом уникальных объектов
может превысить квоту снимка коллекции. Эти случаи остаются в [#1119](https://github.com/IngvarConsulting/unica/issues/1119).

---
id: INV.SOURCE.LARGE-CONFIGURATION-ROOT-VIEW
check:
  - crates/unica-coder/tests/v13_search_integration.rs::canonical_view_reads_configuration_past_eight_mebibytes_with_many_registrations
  - crates/unica-coder/src/infrastructure/v13_large_configuration.rs::registration_cache_reuses_only_the_same_source_revision
gap: https://github.com/IngvarConsulting/unica/issues/1119
---

# Просмотр корня конфигурации не отказывает из-за общего размера XML

`view` читает `Configuration.xml` через удержанный файл и проверяет его до
выдачи полного результата. Общий размер этого файла сам по себе не является
причиной отказа при просмотре корня, ветки метаданных или зарегистрированного
объекта. Повторные запросы к объектам используют индекс регистраций только
для той же ревизии набора исходников; неподтверждённая регистрация не
становится успешным чтением.

Полная работа с большой конфигурацией ещё не достигнута: отдельный большой
XML-токен требует памяти пропорционально его длине, чтение дескриптора объекта
по-прежнему ограничено 8 МиБ, а ветка с большим числом уникальных объектов
может превысить квоту снимка коллекции. Эти случаи остаются в [#1119](https://github.com/IngvarConsulting/unica/issues/1119).

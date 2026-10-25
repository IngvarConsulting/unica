---
id: INV.SOURCE.LARGE-CONFIGURATION-ROOT-VIEW
check:
  - crates/unica-coder/tests/v13_search_integration.rs::canonical_view_reads_configuration_past_eight_mebibytes_with_many_registrations
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::a_descriptor_larger_than_eight_mebibytes_is_read_through_public_view
gap: https://github.com/IngvarConsulting/unica/issues/1119
---

# Просмотр корня конфигурации не отказывает из-за общего размера XML

`view` читает `Configuration.xml` через удержанный файл и проверяет его до
выдачи полного результата. Общий размер этого файла сам по себе не является
причиной отказа при просмотре корня, ветки метаданных или зарегистрированного
объекта. Запросы к объектам используют общий индекс регистраций;
неподтверждённая регистрация не становится успешным чтением. Дескриптор
объекта и его сопутствующие XML-файлы тоже читаются целиком: их размер
не причина отказа.
[Повторное использование разобранного корня и индекса](configuration-root-cache-reuse.md)
имеет отдельные условия.

Полная работа с большой конфигурацией ещё не достигнута: отдельный большой
XML-токен требует памяти пропорционально его длине. Эти случаи остаются в
[#1119](https://github.com/IngvarConsulting/unica/issues/1119).

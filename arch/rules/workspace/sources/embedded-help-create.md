---
id: INV.SOURCE.EMBEDDED-HELP-CREATE
check:
  - crates/unica-coder/src/infrastructure/metadata_operations.rs::typed_add_help_matches_the_retired_help_add_files
  - crates/unica-coder/src/infrastructure/metadata_operations.rs::typed_add_help_is_create_only
  - crates/unica-coder/src/infrastructure/metadata_operations.rs::typed_add_help_preview_writes_nothing
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/metadata.rs::help_create_stages_the_embedded_help_facet
---

# Создание встроенной справки не заменяет существующую

Создание справки владельца добавляет `Ext/Help.xml` и страницу
`Ext/Help/<язык>.html`. Формам владельца без `IncludeHelpInContents`
добавляется это свойство со значением `false`.

Повторное создание отказывает, сохраняя существующую справку и формы.
Предпросмотр показывает план без записи этих файлов.

Запись и повтор проверены через прежний внутренний обработчик метаданных,
который использует общий планировщик справки. Для текущего `help.create`
проверено планирование двух файлов, а не полный публичный вызов.

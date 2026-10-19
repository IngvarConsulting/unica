---
id: INV.SOURCE.EMBEDDED-HELP-CREATE
check:
  - crates/unica-coder/src/infrastructure/metadata_operations.rs::typed_add_help_matches_the_retired_help_add_files
  - crates/unica-coder/src/infrastructure/metadata_operations.rs::typed_add_help_is_create_only
  - crates/unica-coder/src/infrastructure/metadata_operations.rs::typed_add_help_preview_writes_nothing
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/metadata.rs::help_create_stages_the_embedded_help_facet
  - crates/unica-coder/src/infrastructure/daemon/metadata_can_tests.rs::canonical_help_saved_execute_creates_the_facet_and_preserves_neighbors_and_refusals
---

# Создание встроенной справки не заменяет существующую

Создание справки владельца добавляет `Ext/Help.xml` и страницу
`Ext/Help/<язык>.html`. Формам владельца без `IncludeHelpInContents`
добавляется это свойство со значением `false`.

Повторное создание отказывает, сохраняя существующую справку и формы.
Предпросмотр показывает план без записи этих файлов.

Канонический `help.create` проверен через предпросмотр и исполнение сохранённого
токена: оба файла справки, свойство форм, повторный отказ и сохранность соседей.
Изменение прочитанного дескриптора делает план устаревшим; независимая правка
непрочитанного BSL-модуля сохраняется при исполнении.

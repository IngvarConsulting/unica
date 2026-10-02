---
id: INV.SOURCE.CODE-BORROWED-MODULE-STATE
check:
  - crates/unica-coder/src/infrastructure/native_operations/code_module_state_tests.rs::borrowed_code_insert_and_replace_stage_module_state_atomically
  - crates/unica-coder/src/infrastructure/native_operations/code_module_state_tests.rs::borrowed_code_direct_roles_and_common_command_use_their_property
  - crates/unica-coder/src/infrastructure/native_operations/code_module_state_tests.rs::borrowed_code_nested_form_and_command_mark_the_child_descriptor
  - crates/unica-coder/src/infrastructure/native_operations/code_module_state_tests.rs::borrowed_code_root_modules_publish_configuration_state_and_events
  - crates/unica-coder/src/infrastructure/native_operations/code_module_state_tests.rs::borrowed_code_preserves_existing_state_across_xml_layouts_and_repeat
  - crates/unica-coder/src/infrastructure/native_operations/code_module_state_tests.rs::borrowed_code_publication_failure_restores_bsl_descriptor_cache_and_revision
  - crates/unica-coder/src/infrastructure/native_operations/code_module_state_tests.rs::borrowed_code_rejects_malformed_property_state_children_before_staging
  - crates/unica-coder/tests/support/code_module_state.rs::canonical_stdio_code_insert_publishes_borrowed_module_and_state
---

# Запись заимствованного модуля подключает его в расширении

При `code.insert` и `code.replace` в заимствованном объекте расширения
недостающее состояние `PropertyState=Extended` планируется и публикуется
вместе с BSL. Состояние принадлежит ближайшему дескриптору модуля:
для формы свойство называется `Form`, для остальных модулей — платформенным
именем роли. Общий модуль получает `Module=Extended`.

XML остаётся исходным до публикации. Изменение дескриптора несёт отдельное
событие метаданных; для корневого модуля расширения изменяется
`Configuration.xml` и событие относится к `Configuration`.
Общие требования к событиям и откату заданы в
[правиле итоговых событий](../apply-final-effects.md) и
[правиле отката](../retained-apply-rollback.md).

Даже совпадающий BSL может потребовать изменения XML, если состояние
отсутствует. Уже установленное корректное состояние сохраняется побайтно;
некорректное или несовместимое состояние вызывает отказ до публикации.
Собственные объекты расширения и обычная конфигурация не получают состояние
заимствованного объекта.

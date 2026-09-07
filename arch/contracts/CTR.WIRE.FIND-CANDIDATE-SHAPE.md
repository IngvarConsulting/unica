---
id: CTR.WIRE.FIND-CANDIDATE-SHAPE
status: active
governs: product
version: 1
decision: DEC.2026-09-03.FIND-ADDRESS-PATH-DIRECTORY
producer: crates/unica-coder/src/application/v13/find.rs
consumers: [host]
check:
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_name_resolves_to_the_address_and_the_file_that_carries_it
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_file_path_resolves_back_to_its_object_address
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_synonym_resolves_to_its_object
  - crates/unica-coder/src/infrastructure/v13_find.rs::the_directory_holds_objects_and_never_code_symbols_or_inner_nodes
  - crates/unica-coder/src/infrastructure/v13_find.rs::the_directory_refuses_to_grow_past_its_entry_bound
  - crates/unica-coder/src/infrastructure/v13_find.rs::the_directory_observes_cancellation
  - crates/unica-coder/src/infrastructure/v13_find.rs::the_directory_observes_its_operation_deadline
  - crates/unica-coder/src/infrastructure/v13_find.rs::an_external_root_publishes_its_owner_and_never_the_dump_sidecar
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_file_that_is_not_an_owner_descriptor_never_becomes_an_object
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_descriptor_whose_attributes_start_on_a_new_line_is_still_an_object
scope: [wire]
---

# Кандидат find несёт адрес и место объекта в раскладке, но не ревизию

`data.candidates` содержит объекты с полями `at`, `kind`, `title`, `reason` и
`path`. `path` — место объекта в раскладке source set относительно его корня:
файл дескриптора, а у команды её каталог; поле опускается только у записи,
которую раскладка разместить не может. `nearest` появляется только при
отсутствии прямых совпадений.

Результат `find` не несёт `rev`: словарь соответствий адреса и файла не
является снимком ревизии, и клиент не может использовать его как ограду
для `apply`. Ограду по-прежнему выдаёт `view` или предпросмотр `apply`.

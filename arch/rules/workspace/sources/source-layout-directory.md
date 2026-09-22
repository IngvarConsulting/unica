---
id: INV.SOURCE.FIND-IDENTITY-ONLY
check:
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_name_resolves_to_the_address_and_the_file_that_carries_it
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_file_path_resolves_back_to_its_object_address
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_synonym_resolves_to_its_object
  - crates/unica-coder/src/infrastructure/v13_find.rs::the_directory_holds_objects_and_never_code_symbols_or_inner_nodes
  - crates/unica-coder/src/infrastructure/v13_find.rs::the_directory_refuses_to_exceed_resource_bounds
  - crates/unica-coder/src/infrastructure/v13_find.rs::the_directory_observes_cancellation
  - crates/unica-coder/src/infrastructure/v13_find.rs::the_directory_observes_its_operation_deadline
  - crates/unica-coder/src/infrastructure/v13_find.rs::an_external_root_publishes_its_owner_and_never_the_dump_sidecar
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_file_that_is_not_an_owner_descriptor_never_becomes_an_object
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_descriptor_whose_attributes_start_on_a_new_line_is_still_an_object
---

# Справочник раскладки связывает объект с его местом в исходниках

Справочник для поиска имён и разрешения путей связывает имя, синоним
и логический адрес объекта с его файлом или каталогом. Он содержит объекты
и их формы, макеты и команды; методы, области кода, реквизиты и другие
внутренние узлы в него не входят.

В коллекциях объектов, форм и макетов файл с неверным видом или именем
владельца не создаёт запись объекта.
У внешней обработки или отчёта учитывается собственный дескриптор;
`ConfigDumpInfo.xml` и вымышленный путь выгрузки конфигурации не выдаются
за внешний объект.

Построение справочника ограничено числом наборов исходников, числом записей
и суммарным объёмом фактов. Превышение даёт
`provider_limit_exceeded`, отмена — `cancelled`, истечение переданного
срока — `deadline_exceeded`.

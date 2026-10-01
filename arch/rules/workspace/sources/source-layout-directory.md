---
id: INV.SOURCE.FIND-IDENTITY-ONLY
check:
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_name_resolves_to_the_address_and_the_file_that_carries_it
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_file_path_resolves_back_to_its_object_address
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_common_module_file_resolves_to_its_owner_without_becoming_a_name_fact
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_synonym_resolves_to_its_object
  - crates/unica-coder/src/infrastructure/v13_find.rs::the_directory_holds_objects_and_never_code_symbols_or_inner_nodes
  - crates/unica-coder/src/infrastructure/v13_find.rs::the_directory_refuses_to_exceed_resource_bounds
  - crates/unica-coder/src/infrastructure/v13_find.rs::the_directory_observes_cancellation
  - crates/unica-coder/src/infrastructure/v13_find.rs::the_directory_observes_its_operation_deadline
  - crates/unica-coder/src/infrastructure/v13_find.rs::an_external_root_publishes_its_owner_and_never_the_dump_sidecar
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_file_that_is_not_an_owner_descriptor_never_becomes_an_object
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_descriptor_whose_attributes_start_on_a_new_line_is_still_an_object
  - crates/unica-coder/src/infrastructure/v13_find.rs::large_configuration_descriptor_still_has_a_layout_address
  - crates/unica-coder/src/infrastructure/v13_find.rs::point_lookup_ignores_the_search_collection_entry_limit
  - crates/unica-coder/src/infrastructure/v13_find.rs::point_lookup_common_module_file_requires_target_and_owner_without_full_directory
  - crates/unica-coder/src/infrastructure/v13_find.rs::point_lookup_refuses_when_target_name_is_outside_the_descriptor_sample
  - crates/unica-coder/src/infrastructure/v13_find.rs::point_lookup_refuses_an_existing_broken_target_descriptor
  - crates/unica-coder/src/infrastructure/v13_find.rs::point_lookup_refuses_an_existing_broken_external_target_descriptor
  - crates/unica-coder/src/infrastructure/daemon/mod.rs::resolve_path_ignores_the_full_directory_entry_budget
  - crates/unica-coder/tests/platform/v13_resolve_target_isolation.rs::resolve_path_checks_only_target_and_necessary_owner_on_public_mcp
  - crates/unica-coder/tests/platform/v13_resolve_target_isolation.rs::resolve_absolute_xml_ignores_a_broken_foreign_target_and_reports_ambiguous_alias
  - crates/unica-coder/tests/platform/v13_resolve_target_isolation.rs::resolve_path_does_not_inherit_the_search_source_set_limit
gap: https://github.com/IngvarConsulting/unica/issues/976
---

# Справочник раскладки связывает объект с его местом в исходниках

Справочник для поиска имён связывает имя, синоним и логический адрес объекта
с его файлом или каталогом. Разрешение конкретного пути доказывает эту же
связь точечно по запрошенному объекту и необходимым владельцам, не строя
полный справочник. Эти операции охватывают объекты
и их формы, макеты и команды; методы, области кода, реквизиты и другие
внутренние узлы в него не входят.

В коллекциях объектов, форм и макетов файл с неверным видом или именем
владельца не создаёт запись объекта.
Путь `CommonModules/<Имя>/Ext/Module.bsl` разрешается в общий модуль только
при наличии обычного файла модуля и подтверждённого дескриптора владельца;
путь ответа указывает на запрошенный файл модуля.
У внешней обработки или отчёта учитывается собственный дескриптор;
`ConfigDumpInfo.xml` и вымышленный путь выгрузки конфигурации не выдаются
за внешний объект.

Построение полного справочника для поиска имён ограничено числом наборов
исходников, числом записей, размером перечисляемой коллекции и суммарным
объёмом фактов. Разрешение конкретного пути не наследует ограничения числа
наборов полного справочника, его совокупные ограничения или размер перечисляемой
коллекции. Подтверждение владельца по-прежнему берёт
начало XML-дескриптора; если существующий целевой дескриптор не подтверждён
этой выборкой, вызов отказывает, а не возвращает ложный `not_found`.
Полноценное потоковое подтверждение XML остаётся открытым пробелом.
Превышение применимого предела даёт
`provider_limit_exceeded`, отмена — `cancelled`, истечение переданного
срока — `deadline_exceeded`.

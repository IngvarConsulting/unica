---
id: INV.SOURCE.RESOLVED-RESOURCE-IDENTITY
check:
  - crates/unica-coder/tests/platform/v13_resolve_resources.rs::resolve_external_form_resources_round_trip_all_three_file_kinds
  - crates/unica-coder/tests/platform/v13_resolve_resources.rs::resolve_external_form_resources_require_registered_matching_owner_and_form
  - crates/unica-coder/tests/platform/v13_resolve_resources.rs::resolve_external_form_resources_never_follow_linked_content
  - crates/unica-coder/tests/platform/v13_resolve_resources.rs::resolve_form_metadata_survives_absent_local_content_without_substituting_it
  - crates/unica-coder/tests/platform/v13_resolve_resources.rs::resolve_common_form_resources_preserve_deepest_root_and_ambiguity
  - crates/unica-coder/tests/platform/v13_resolve_resources.rs::resolve_common_form_resources_never_follow_content_or_metadata_links
  - crates/unica-coder/tests/platform/v13_resolve_resources.rs::resolve_resource_refuses_mismatched_descriptor_identity_on_public_mcp
  - crates/unica-coder/tests/platform/v13_resolve_resources.rs::resolve_form_and_module_paths_preserve_the_exact_resource_on_public_mcp
  - crates/unica-coder/tests/platform/v13_resolve_resources.rs::resolve_resources_refuse_unregistered_and_absent_targets_on_public_mcp
  - crates/unica-coder/tests/platform/v13_resolve_resources.rs::resolve_resources_keep_source_prefix_and_deepest_root_selection_on_public_mcp
  - crates/unica-coder/tests/platform/v13_resolve_resources.rs::resolve_resources_do_not_follow_linked_files_or_directories_on_public_mcp
  - crates/unica-coder/tests/platform/v13_resolve_resources.rs::resolve_resource_prefix_skips_a_foreign_linked_collection_on_public_mcp
  - crates/unica-coder/src/infrastructure/v13_find.rs::point_lookup_routes_owner_module_roles_through_the_platform_profile
  - crates/unica-coder/src/infrastructure/v13_find.rs::a_common_module_file_resolves_to_its_owner_without_becoming_a_name_fact
  - crates/unica-coder/src/infrastructure/v13_find.rs::an_external_root_publishes_its_owner_and_never_the_dump_sidecar
---

# Разрешение пути сохраняет ресурс формы или модуля

У формы три разных ресурса: структура `Forms/<форма>/Ext/Form.xml`
(или `CommonForms/<форма>/Ext/Form.xml`), декларация `Forms/<форма>.xml`
(или `CommonForms/<форма>.xml`)
и модуль `Ext/Form/Module.bsl`. Они разрешаются соответственно в адрес формы,
её `Metadata` и `Module.Form`; обратное разрешение возвращает тот же локальный
ресурс. Вложенный элемент указывает на содержащий XML, а путь всего XML
не восстанавливает адрес элемента. Отсутствующее локальное содержимое
заимствованной формы не заменяется декларацией или файлом базового набора;
локальная `Metadata` доступна независимо от содержимого.

У именованной формы внешней обработки или отчёта декларация лежит под
`<владелец>/Forms/<форма>.xml`, структура и модуль — под
`<владелец>/Forms/<форма>/Ext/`. Эта обратимость не добавляет разрешение
прочих модулей внешнего объекта. Внешний объект доказывается собственным
дескриптором; `ConfigDumpInfo.xml` и вымышленная раскладка конфигурации его
не заменяют.

`Ext/<Роль>Module.bsl` владельца разрешается в `Module.<Роль>`, только если
раскладка и профиль платформы допускают роль для этого владельца, включая
корень конфигурации и общую или вложенную команду. Общий модуль требует
обычного файла `CommonModules/<Имя>/Ext/Module.bsl` и подтверждённого
дескриптора. Путь ответа сохраняет запрошенный файл, не XML владельца.

Регистрацию владельца и формы подтверждает тот же логический reader, что
`view`; вид и имя дескриптора должны совпадать. Поддержаны полный путь
от корня набора, префикс workspace и абсолютный путь по
[правилу точечного разрешения](source-path-resolution.md).
Физические двойники разных наборов дают неоднозначность до проверки
регистрации, включая незарегистрированный двойник. Файлы и каталоги-ссылки
не обходятся, а посторонняя связанная коллекция не мешает доказанной цели.

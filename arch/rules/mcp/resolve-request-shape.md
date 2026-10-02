---
id: INV.WIRE.RESOLVE-EXACT-BRIDGE
check:
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_catalog_locks_the_eight_domain_contracts_without_publishing_them
  - crates/unica-coder/src/application/v13/resolve.rs::resolve_takes_exactly_one_side_of_the_bridge
  - crates/unica-coder/src/application/v13/resolve.rs::absent_lines_are_named_rather_than_omitted
  - crates/unica-coder/src/infrastructure/daemon/mod.rs::resolve_path_ignores_the_full_directory_entry_budget
  - crates/unica-coder/tests/platform/v13_resolve_target_isolation.rs::resolve_path_checks_only_target_and_necessary_owner_on_public_mcp
  - crates/unica-coder/tests/platform/v13_resolve_target_isolation.rs::resolve_absolute_xml_ignores_a_broken_foreign_target_and_reports_ambiguous_alias
  - crates/unica-coder/tests/platform/v13_resolve_target_isolation.rs::resolve_relative_source_prefix_skips_foreign_linked_collections_on_public_mcp
  - crates/unica-coder/tests/platform/v13_resolve_target_isolation.rs::resolve_absolute_path_uses_the_deepest_admitted_source_root
  - crates/unica-coder/tests/platform/v13_resolve_target_isolation.rs::resolve_path_succeeds_above_the_full_directory_byte_budget
  - crates/unica-coder/tests/platform/v13_resolve_target_isolation.rs::resolve_path_does_not_inherit_the_search_source_set_limit
  - crates/unica-coder/src/infrastructure/daemon/mod.rs::name_search_reports_an_injected_local_read_fault_through_the_live_daemon
  - crates/unica-coder/src/infrastructure/v13_find.rs::point_lookup_preserves_full_relative_paths_and_checks_every_source_before_not_found
  - crates/unica-coder/src/infrastructure/v13_find.rs::point_lookup_refuses_ambiguous_paths_across_admitted_sources
  - crates/unica-coder/src/infrastructure/v13_find.rs::point_lookup_uses_the_full_layout_path_even_with_a_shorter_suffix
  - crates/unica-coder/src/infrastructure/v13_find.rs::absolute_common_module_path_skips_a_broken_alias_in_another_source
  - crates/unica-coder/src/infrastructure/v13_find.rs::absolute_lookup_keeps_distinct_source_roots_with_different_case
  - crates/unica-coder/src/infrastructure/v13_find.rs::absolute_lookup_accepts_a_physical_unicode_alias_of_the_source_root
  - crates/unica-coder/src/infrastructure/v13_find.rs::windows_drive_and_unc_prefixes_keep_the_same_root_anchor_shape
  - crates/unica-coder/src/infrastructure/v13_find.rs::absolute_lookup_refuses_a_replaced_requested_root_ancestor
  - crates/unica-coder/src/infrastructure/v13_find.rs::absolute_lookup_skips_a_foreign_root_at_the_target_files_depth
  - crates/unica-coder/src/infrastructure/v13_find.rs::absolute_lookup_distinguishes_a_file_from_a_case_folded_foreign_root
  - crates/unica-coder/src/infrastructure/v13_find.rs::absolute_lookup_does_not_open_a_damaged_case_distinct_foreign_parent
  - crates/unica-coder/src/infrastructure/v13_find.rs::relative_source_prefix_skips_linked_collections_in_another_source
  - crates/unica-coder/src/infrastructure/v13_find.rs::point_lookup_checks_the_owner_of_a_nested_object
  - crates/unica-coder/src/infrastructure/v13_find.rs::point_lookup_nested_path_ignores_an_unrelated_linked_owner_directory
  - crates/unica-coder/src/infrastructure/v13_find.rs::linked_proven_owner_directory_refuses_instead_of_hiding_nested_names
gap: https://github.com/IngvarConsulting/unica/issues/976
---

# Запрос resolve выбирает одну сторону адресного моста

`unica.resolve` принимает ровно один аргумент: логический адрес `at`
или путь `path`. Два аргумента вместе и отсутствие обоих дают `bad_value`.

Форма расположения явно называет наличие строк: `lines.state` равен
`range` с границами `from` и `to` либо `notLineBased`. Поле не опускается,
когда диапазона нет. Эти проверки закрепляют разбор запроса и сериализацию
расположения. Они не доказывают существование указанного узла и правильность
его диапазона в исходниках.

При успешном ответе указанный предмет существует; результат сообщает его
адрес, вид и действительное место в исходниках. Отсутствующий предмет даёт
`not_found`. Для метода или области BSL диапазон относится именно к файлу
модуля, а не к XML-дескриптору его владельца. У древовидного ресурса диапазон
не выдумывается. Эти гарантии текущего вызова требуют проверки из `gap`.

При `path` проверяются запрошенный объект и необходимые владельцы. Абсолютный
путь внутри допущенного корня выбирает самый глубокий подходящий набор
исходников, когда допущенные корни вложены; его связь
с корнем и объектом подтверждается физической идентичностью и правилами имён
каталогов. Путь вне всех допущенных корней не указывает на объект.
Относительный путь с доказанным
префиксом корня также выбирает свой набор; без такого префикса проверяются
подходящие кандидаты всех допущенных наборов. Точечный поиск не наследует
ограничения числа наборов исходников полного поиска. Размер общего справочника имён
и повреждение постороннего дескриптора не препятствуют ответу о запрошенном
объекте. Запрос содержит полный путь из раскладки, возможно с префиксом
рабочей области; укороченное имя файла не является таким путём. Отсутствующую
часть раскладки нельзя отбросить ради совпадения с более коротким чужим путём.
Разные объекты по одному относительному пути дают отказ вместо произвольного
выбора. `not_found` возможен лишь после проверки всех относящихся к запросу
кандидатов.

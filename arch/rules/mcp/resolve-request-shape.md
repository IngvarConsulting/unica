---
id: INV.WIRE.RESOLVE-EXACT-BRIDGE
check:
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_catalog_locks_the_eight_domain_contracts_without_publishing_them
  - crates/unica-coder/src/application/v13/resolve.rs::resolve_takes_exactly_one_side_of_the_bridge
  - crates/unica-coder/src/application/v13/resolve.rs::absent_lines_are_named_rather_than_omitted
  - crates/unica-coder/src/infrastructure/daemon/mod.rs::resolve_path_ignores_the_full_directory_entry_budget
  - crates/unica-coder/src/infrastructure/daemon/mod.rs::name_search_reports_an_injected_local_read_fault_through_the_live_daemon
  - crates/unica-coder/src/infrastructure/v13_find.rs::point_lookup_preserves_full_relative_paths_and_checks_every_source_before_not_found
  - crates/unica-coder/src/infrastructure/v13_find.rs::point_lookup_refuses_ambiguous_paths_across_admitted_sources
  - crates/unica-coder/src/infrastructure/v13_find.rs::point_lookup_prefers_the_longest_stored_path
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

При `path` проверяются возможные совпадения и необходимые владельцы в каждом
допущенном наборе исходников. Размер общего справочника имён и повреждение
дескриптора вне этого доказательства не препятствуют ответу о запрошенном
объекте. Запрос содержит полный путь из раскладки, возможно с префиксом
рабочей области; укороченное имя файла не является таким путём. Если несколько
записей соответствуют хвосту запроса, побеждает самая длинная раскладка;
равноточные разные объекты дают отказ вместо произвольного выбора.
`not_found` возможен лишь после проверки всех кандидатов.

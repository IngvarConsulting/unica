---
id: INV.WIRE.OPERATION-TARGET-ADDRESS
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::retired_member_arguments_route_next_to_the_member_address
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::a_retired_scope_call_recovers_through_next_to_a_published_tabular_attribute
  - crates/unica-coder/src/domain/apply.rs::member_operations_apply_at_the_member_address_not_at_the_owner
  - crates/unica-coder/src/domain/apply.rs::every_retired_argument_entry_builds_a_routable_member_address
  - crates/unica-coder/src/domain/apply.rs::a_duplicate_argument_is_refused_with_the_node_its_value_named
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::retired_duplicate_arguments_route_next_to_the_addressed_node
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::code_replace_refuses_a_method_selector_and_names_the_method_address
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/dcs_mxl.rs::a_query_is_addressed_through_its_dataset_and_retired_spellings_refuse
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/form_resource.rs::form_and_child_subsystem_names_come_only_from_the_address
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/dcs_mxl.rs::a_data_set_is_found_by_name_inside_unions
  - crates/unica-coder/src/domain/apply.rs::a_subnode_named_by_an_argument_is_refused_with_its_address
  - crates/unica-coder/src/infrastructure/daemon/v13_read_modes.rs::operation_dictionary_tests::a_retired_operation_contract_names_its_replacement
  - crates/unica-coder/src/infrastructure/daemon/v13_read_modes.rs::operation_dictionary_tests::a_retired_operation_route_follows_roles_and_inherited_content
  - crates/unica-coder/src/domain/apply.rs::a_renamed_operation_target_respects_the_item_role
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::retired_subnode_arguments_route_next_to_the_subnode_address
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/form_resource.rs::right_set_grants_lists_and_restricts_rights_on_the_staged_document
  - crates/unica-coder/src/domain/apply.rs::dcs_schema_nodes_and_structure_items_take_their_subject_from_the_address
  - crates/unica-coder/src/domain/apply.rs::a_repeated_schema_name_routes_next_to_the_branch_and_every_match
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/dcs_mxl.rs::a_structure_item_is_found_by_its_named_ancestors
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::retired_dcs_container_arguments_route_next_to_the_structure_item
  - crates/unica-coder/src/infrastructure/v13_read_projection.rs::dcs_structure_tests::repeated_total_paths_stay_rows_and_are_not_chosen
  - crates/unica-coder/src/domain/apply.rs::a_form_item_or_event_named_by_an_argument_is_refused_with_its_address
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::retired_form_arguments_route_next_to_the_item_or_event_address
---

# Операция изменяет адресованный узел и объясняет замену снятой формы вызова

Изменение или удаление самостоятельно адресуемого узла выполняется по его
адресу, а не по адресу владельца с выбором цели в аргументах. Это относится
к членам метаданных, подузлам СКД, правам роли, областям макета, методам и
узлам формы. Подробный контракт выбирает адрес и маршрут, в том числе при
одинаковом имени операции у разных семейств. Операция не пишет в другой узел.

Снятые поля и их адресные замены имеют закрытый перечень
[`RETIRED_ARGUMENTS`](../../../crates/unica-coder/src/domain/apply.rs).
Такое поле получает `bad_value` до планирования и до отказа «не тот узел»,
если из него строится исполнимая цель. Первый `next` ведёт к `Can` названного
узла, второй — к ветви его вида. Значение, повторяющее адрес вызова, получает
указание убрать поле. Идентификатор вместо адресного имени ведёт к коллекции,
где доступны имя и `id`; неизвестный или неразрешимый снятый ключ также
отклоняется контрактом либо планировщиком с `bad_value`.

Снятые операции не становятся синонимами. Отказ и чтение их контракта ведут
к заменившей операции на ближайшем исполнимом узле с учётом роли и
унаследованного содержимого. Цель другого семейства и неподходящий по роли
узел не подставляются. Если подходящего предка нет, отказ называет форму
адреса и словарь узла. Создающую операцию задаёт
[контракт родителя](operation-creation-parent.md).

Запрос СКД адресуется через `DataSet.<набор>.Query`, а операции набора —
самим набором. Снятый путь `Template.<макет>.Query.<набор>` направляет к
соответствующей цели. При исправлении снятого `dataSet` выбирается одноимённый
сегмент адреса, затем набор по имени в схеме, включая объединения, затем
верхний уровень. При нескольких совпадениях `next` называет ветвь и все
кандидаты, не выбирая один за вызывающего.

Контейнер настроек задаётся адресом варианта или именованного элемента
структуры; замена всей структуры и параметры данных относятся только к
варианту. Снятые `group`/`parent` и `group` параметров данных отклоняются.
Безымянные элементы прозрачны для адреса потомков; имя выбирает только
единственный элемент на своём уровне. Повторённые имена ведут к ветви и
всем кандидатам. Итоги с повторённым `dataPath` остаются строками без адреса.

Имя предопределённого элемента уникально во всём объекте на любой глубине:
создание или переименование на занятое имя отклоняется. Право роли изменяется
только по каноническому `<Вид>_<Имя>`, хотя чтение допускает короткий
однозначный псевдоним. Точки в адресных именах экранируются по
[грамматике адресов](qualified-logical-address.md).

Ключи строк без устойчивой адресной идентичности (отбор, выбор, порядок,
оформление, связь наборов) остаются аргументами. Текстовый `selector.anchor`
также остаётся аргументом; вставку относительно метода задаёт
[правило позиции кода](../workspace/sources/code-insert-position.md).
Ограничения ролей формы задаёт [правило операций формы](../workspace/sources/form-operation-targets.md).

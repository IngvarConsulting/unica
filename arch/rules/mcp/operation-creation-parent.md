---
id: INV.WIRE.OPERATION-CREATION-PARENT
check:
  - crates/unica-coder/src/domain/apply.rs::a_retired_creation_names_the_parent_operation
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/form_resource.rs::role_create_contract_preserves_typed_names_and_rejects_unknown_args
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::an_area_is_written_only_where_it_exists_and_created_only_by_area_add
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::a_missing_rights_entry_or_area_routes_next_to_the_parent
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::a_missing_target_routes_next_to_the_parent_dictionary
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/form_resource.rs::form_add_then_element_and_event_edits_compose_in_one_batch
---

# Новый узел создаёт операция его родителя

Создание адресуемого узла выполняет операция родителя с именем нового узла
в аргументах. `*.set` и `*.remove` работают с существующим узлом: опечатка
в адресе не создаёт объект. При доказанном отсутствии цели `not_found`
содержит `next` к родителю, публикующему создающую операцию. Отсутствие файла
ресурса у существующего узла или неизвестное существование такого перехода
не разрешают.

`right.add` на роли задаёт начальный набор прав, `area.add` на макете —
начальные ячейки, `form.add` на владельце — форму. Повторное имя у `right.add`
и `area.add` даёт `bad_value` с адресом для изменения. `right.set` и `mxl.set`
на отсутствующем узле отказывают; их снятые аргументы дополнительно называют
создающую операцию родителя и дают третьим `next` её подробный контракт.

`form.create` не входит в реестр и получает `bad_value` с маршрутом к
`form.add` владельца. Контракты создания и изменения читаются по
[правилу словаря операций](operation-dictionary.md); прежние формы вызова
не принимаются как псевдонимы.

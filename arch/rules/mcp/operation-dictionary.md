---
id: INV.WIRE.V13-CAN-DICTIONARY
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::support_can_contract_examples_preview_without_source_changes
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/form_resource.rs::support_contracts_and_productive_parser_agree_on_closed_arguments
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_catalog_locks_the_eight_domain_contracts_without_publishing_them
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_refusals_answer_one_diagnostics_channel_from_the_closed_code_set
  - crates/unica-coder/src/infrastructure/daemon/v13_read_modes.rs::every_node_answer_carries_its_dictionary_and_keeps_its_own_slots
  - crates/unica-coder/src/infrastructure/daemon/v13_read_modes.rs::a_node_without_a_computed_dictionary_has_no_can_and_its_operation_is_refused
  - crates/unica-coder/src/infrastructure/daemon/v13_read_modes.rs::detailed_dcs_can_publishes_structured_operands_and_refuses_retired_schema_input
  - crates/unica-coder/src/infrastructure/daemon/v13_read_modes.rs::diff_compares_source_backed_data_without_the_computed_dictionary
  - crates/unica-coder/src/application/v13/diff.rs::diff_ignores_computed_view_sections
  - crates/unica-coder/src/domain/address.rs::a_can_segment_takes_the_rest_of_the_address_as_the_operation_name
  - crates/unica-coder/src/infrastructure/daemon/server.rs::a_retired_read_filter_routes_next_to_the_address_that_replaced_it
  - crates/unica-coder/src/infrastructure/daemon/server.rs::an_operation_address_outside_view_routes_next_to_view
  - crates/unica-coder/src/domain/apply.rs::applicability_follows_the_route_not_the_terminal_kind
  - crates/unica-coder/src/domain/apply.rs::a_misplaced_operation_names_the_nearest_node_where_it_applies
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::honest_can_on_the_main_corpus_workspace
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::honest_can_on_the_dcs_workspace
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::honest_can_on_the_mxl_workspace
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::honest_can_on_the_code_and_extension_workspace
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::honest_can_on_external_sources
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::fixture_coverage_reaches_every_route_class
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::a_misplaced_operation_routes_next_to_the_node_where_it_applies
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::without_an_executable_ancestor_the_refusal_names_the_address_shape
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::honest_can_on_the_predefined_workspace
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/metadata.rs::member_operation_contracts_and_the_planner_agree_on_structure
  - crates/unica-coder/src/domain/operation_contract.rs::dcs_contract_defaults_aliases_and_roles_match_the_typed_boundary
  - crates/unica-coder/src/infrastructure/daemon/v13_read_modes.rs::operation_dictionary_tests::a_shared_operation_name_publishes_the_contract_of_the_addressed_node
  - crates/unica-coder/src/domain/operation_contract.rs::role_contracts_publish_the_entry_shape
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/form_resource.rs::role_create_contract_preserves_typed_names_and_rejects_unknown_args
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::every_role_family_contract_builds_a_preview_without_source_writes
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::honest_can_on_form_items_and_events
---

# Словарь узла и контракт операции соответствуют принимаемому apply

Ответ `view` узла с вычисляемым словарём содержит `can`: имя `op`, форму
`args` и `implemented`. Состав зависит от маршрута, вида узла и набора
исходников и строится из того же реестра, который проверяет `apply`.
Имя в словаре само по себе не означает реализованность. Схема `apply`
принимает имя строкой, без второго перечисления операций или их схем.

Подробный контракт читается через `view` по
[адресу операции](qualified-logical-address.md). Узел `Can` возвращает в
`props` имя, форму аргументов, реализованность и контракт, суженный до цели.
Отсутствующая операция или пустое имя дают `bad_value` с `next` к узлу.
Если словарь не вычисляется, `can` отсутствует, а его операция отвечает
`unsupported_section`; так же отвечает реализованная операция без
опубликованного подробного контракта. Вычисленный словарь может быть пустым.

Состав `can`, предварительная проверка и разбор цели планировщиком совпадают.
Контракт и планировщик одинаково принимают структуру аргументов с учётом
вида владельца и табличной части. Перекрёстные ограничения полей, например
соответствие значения заполнения явно заданному типу, проверяет планировщик.
Ошибка аргументов называет ожидаемый ключ. Неприменимая операция ведёт к
словарю исполнимой цели; если её нет, отказ называет нужную форму адреса и
ведёт к словарю читаемого узла, иначе к корню набора.

Адрес `Can` читается только `view`. `check`, `resolve`, `apply` (во всех
позициях адреса) и `search.scope` отвечают `bad_value`/`fixCall` с `next`
к `view` того же адреса. `diff` исключает вычисляемый `can` из сравнения и
отклоняет адрес операции с `target_kind_unsupported`. Снятый `filter` у
`view` и `diff` получает отказ неизвестного аргумента с адресом замены в `next`.

Цель изменения задаёт [правило адреса операции](operation-target-address.md),
создание — [правило родителя](operation-creation-parent.md).
Проверки честности словаря сопоставляют проекции с предварительной проверкой,
разбором семейства, ограничениями формата и файлами плана. Намеренное
сужение применимости должно быть явно названо.

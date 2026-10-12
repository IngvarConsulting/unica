---
id: INV.WIRE.V13-CAN-DICTIONARY
check:
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_catalog_locks_the_eight_domain_contracts_without_publishing_them
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_refusals_answer_one_diagnostics_channel_from_the_closed_code_set
  - crates/unica-coder/src/infrastructure/daemon/v13_read_modes.rs::sections_keep_identity_and_only_selected_optional_slots
  - crates/unica-coder/src/infrastructure/daemon/v13_read_modes.rs::uncomputed_sections_answer_typed_unsupported_section
  - crates/unica-coder/src/infrastructure/daemon/v13_read_modes.rs::diff_filter_selects_closed_json_pointer_prefixes
  - crates/unica-coder/src/application/v13/diff.rs::diff_sections_refuse_computed_view_sections
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
gap: https://github.com/IngvarConsulting/unica/issues/1358
---

# Словарь операций узла появляется по явному запросу can

`unica.view` публикует словарь операций только при запросе
`filter.sections: ["can"]`. Состав зависит от вида и маршрута узла
(дерева, которому узел принадлежит: метаданные, форма, СКД, табличный
документ, модуль, роль, интерфейс, XDTO) и вида набора исходников.
Он строится из реестра, которым проверяется `apply`. Элемент называет `op`,
ожидаемую форму `args` и признак `implemented`; наличие имени в словаре
само по себе не означает реализованность. Схема `apply` принимает имя
операции строкой и не дублирует словарь перечислением имён или объединением
ветвей операций.

Для вида без вычисляемого словаря запрос даёт `unsupported_section`.
Словарь содержит ровно те операции, которые на этом узле принимают
предварительная проверка `apply` и разбор цели планировщиком; операция
не пишет в другой узел. Ошибка формы аргументов называет ожидаемый ключ.
Неизвестная операция направляет к `can` того же узла, а неприменимая —
к `can` узла, где она исполнима. Если такого узла нет, отказ называет
форму нужного адреса, а `next` ведёт к `can` того же узла, когда тот
читается, иначе к корню набора. `diff` не принимает вычисляемые секции
`can` и `limits`.

До переноса операций на адреса подузлов (пакеты П1–П6 зонта #1358)
обещание «не пишет в другой узел» выполнено не для всех операций:
те, что ещё называют подузел аргументом (записи таблицы снятых аргументов
в реестре, например `values.name` у `tabularSection.set` или `scope`
у `attribute.add`), пишут в подузел, названный аргументом, а не адресом.

Проверки проходят настоящий вызов `view` для объекта метаданных и
отдельно проверяют проекции корня, модуля, неизвестного вида и операции
без реализации. Двусторонняя сверка обходит проекции чтения фикстур
приёмочного корпуса и для представительных узлов каждой пары «маршрут,
вид» сравнивает словарь с предварительной проверкой, разбором семейства
в обход реестра (расхождение допустимо только как названное намеренное
сужение), фильтром формата макета в планировщике и файлами плана.

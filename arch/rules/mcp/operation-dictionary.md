---
id: INV.WIRE.V13-CAN-DICTIONARY
check:
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
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::retired_member_arguments_route_next_to_the_member_address
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::a_retired_scope_call_recovers_through_next_to_a_published_tabular_attribute
  - crates/unica-coder/src/domain/apply.rs::member_operations_apply_at_the_member_address_not_at_the_owner
  - crates/unica-coder/src/domain/apply.rs::every_retired_argument_entry_builds_a_routable_member_address
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/metadata.rs::member_operation_contracts_and_the_planner_agree_on_structure
  - crates/unica-coder/src/domain/apply.rs::a_duplicate_argument_is_refused_with_the_node_its_value_named
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::retired_duplicate_arguments_route_next_to_the_addressed_node
  - crates/unica-coder/src/domain/operation_contract.rs::dcs_contract_defaults_aliases_and_roles_match_the_typed_boundary
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::code_replace_refuses_a_method_selector_and_names_the_method_address
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/dcs_mxl.rs::a_query_is_addressed_through_its_dataset_and_retired_spellings_refuse
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/form_resource.rs::form_and_child_subsystem_names_come_only_from_the_address
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/dcs_mxl.rs::a_data_set_is_found_by_name_inside_unions
gap: https://github.com/IngvarConsulting/unica/issues/1358
---

# Словарь операций приходит в ответе узла, контракт операции — по её адресу

Ответ `unica.view` каждого узла, для вида которого словарь вычисляется,
содержит `can`. Состав зависит от вида и маршрута узла
(дерева, которому узел принадлежит: метаданные, форма, СКД, табличный
документ, модуль, роль, интерфейс, XDTO) и вида набора исходников.
Он строится из реестра, которым проверяется `apply`. Элемент называет `op`,
ожидаемую форму `args` и признак `implemented`; наличие имени в словаре
само по себе не означает реализованность. Схема `apply` принимает имя
операции строкой и не дублирует словарь перечислением имён или объединением
ветвей операций.

Подробный контракт аргументов одной операции читается адресом
`<узел>.Can.<op>`. Сегмент `Can` последний, и имя операции занимает весь
остаток адреса вместе с точкой (`main:Catalog.Валюты.Can.attribute.add`);
у корня конфигурации это `<набор>:Configuration.Can.<op>`. Ответ — узел вида
`Can` с адресом операции, в `props` которого имя, форма `args`, признак
`implemented` и контракт, суженный до этого узла. Неизвестная операция
и операция, которой нет в словаре узла, получают `bad_value`, как и `Can` без
имени операции; `next` ведёт к самому узлу. У вида без вычисляемого словаря
ответ узла не содержит `can`, а адрес его операции даёт `unsupported_section`;
тот же отказ получает реализованная операция, подробный контракт которой
ещё не опубликован. Вычисленный словарь может быть пустым: на узле ничего
не применимо. Адрес операции читает только `view`: `check` и `resolve`
(`at`), `apply` (`at` вызова, `at` операции и `args.at` операции) и `search`
(`scope`) отвечают на него `bad_value` с исходом `fixCall`, сообщением, что это
адрес словаря операций, и `next` к `unica.view` с тем же адресом.
Аргумента `filter` у `view` и `diff` нет: вызов с ним получает отказ
неизвестного аргумента, и `next` называет адрес, который заменил фильтр.

Словарь содержит ровно те операции, которые на этом узле принимают
предварительная проверка `apply` и разбор цели планировщиком; операция
не пишет в другой узел. Ошибка формы аргументов называет ожидаемый ключ.
Неизвестная операция направляет к `can` того же узла, а неприменимая —
к `can` узла, где она исполнима. Если такого узла нет, отказ называет
форму нужного адреса, а `next` ведёт к `can` того же узла, когда тот
читается, иначе к корню набора. `diff` сравнивает данные узлов без
вычисляемого `can`, а адрес операции `<узел>.Can.<op>` отклоняет
`target_kind_unsupported`.

Члены объекта метаданных адресуются сами: табличная часть, измерение,
ресурс, значение перечисления, графа журнала, команда, макет
и предопределённый элемент изменяются и удаляются по своему адресу,
а реквизит табличной части добавляется по адресу части. Аргумент, которым
такой вызов прежде называл член (`scope.tabularSection`, `values.name`,
`values.id`), получает отказ `bad_value` до вердикта «не тот узел». Первый
`next` ведёт к `can` члена, второй — к ветви членов этого вида у владельца,
где видны существующие имена. Для предопределённого элемента, названного
идентификатором, `next` ведёт к элементам его коллекции: у каждого видны имя
и `id`. Имя предопределённого элемента служит адресом, поэтому оно уникально
в объекте на любой глубине; добавление или переименование на занятое имя
отвергается. Подробный контракт операции члена (`<член>.Can.<op>`) строится
для этого узла: поля, допустимость которых зависит от вида владельца
или табличной части, сужены до него. На этом узле контракт и разбор
планировщиком одинаково принимают и отвергают структуру аргументов.
Перекрёстные правила полей — значение заполнения требует явный тип
и соответствует ему — схема не выражает; их проверяет только планировщик.

Аргумент, который повторял адрес вызова или называл узел в обход него, снят:
набор у `query.set` и `query.patch` (`values.dataSet`), вариант у
`structure.set` и `structure.patch` (`values.variant`), имя формы у
`form.create` (`values.name`), дочерняя подсистема у `childSubsystem.remove`
(`items[].name`, элемент-строка `items[]`, `values.name`) и метод у
`code.replace` (`selector.method`). Если из значения строится адрес, где
операция исполнима, вызов получает `bad_value` до планирования, даже когда
операция исполнима и на адресе вызова. Первый `next` ведёт к узлу, который
называло значение, второй — к ветви узлов этого вида; к форме, которую
`form.create` только создаёт, `next` не ведёт, остаётся ветвь форм. Если
значение повторяло адрес вызова, отказ говорит снять аргумент. Иначе
снятый ключ отвергают контракт или разбор планировщиком, тоже `bad_value`.
Набор, названный снятым `dataSet`, — это сегмент адреса вызова с тем же
именем, иначе набор, найденный по имени в схеме макета (в том числе член
объединения), иначе набор верхнего уровня. Операции запроса исполнимы
только на запросе набора `DataSet.<набор>.Query`; вызов на самом наборе
получает отказ «не тот узел» с `next` к `can` его запроса. Прежний адрес
`Template.<макет>.Query.<набор>` отвергается с `next` к запросу этого
набора, а для операций набора — к самому набору; остальные операции набора
на узле запроса не исполнимы.
Дочерняя подсистема исключается из родителя только по своему адресу.
Текстовый якорь `selector.anchor` у `code.replace` остаётся аргументом.

До переноса остальных операций на адреса подузлов (пакеты П3–П6 зонта
#1358) обещание «не пишет в другой узел» выполнено не для всех операций:
те, что ещё называют подузел аргументом (записи таблицы снятых аргументов
в реестре, например `values.element` у `event.bind`), пишут в подузел,
названный аргументом, а не адресом.

Проверки проходят настоящий вызов `view` для объекта метаданных и
отдельно проверяют проекции корня, модуля, неизвестного вида и операции
без реализации. Двусторонняя сверка обходит проекции чтения фикстур
приёмочного корпуса и для представительных узлов каждой пары «маршрут,
вид» сравнивает словарь с предварительной проверкой, разбором семейства
в обход реестра (расхождение допустимо только как названное намеренное
сужение), фильтром формата макета в планировщике и файлами плана.

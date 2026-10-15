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
  - crates/unica-coder/src/domain/apply.rs::a_subnode_named_by_an_argument_is_refused_with_its_address
  - crates/unica-coder/src/domain/apply.rs::a_retired_creation_names_the_parent_operation
  - crates/unica-coder/src/infrastructure/daemon/v13_read_modes.rs::operation_dictionary_tests::a_retired_operation_contract_names_its_replacement
  - crates/unica-coder/src/infrastructure/daemon/v13_read_modes.rs::operation_dictionary_tests::a_shared_operation_name_publishes_the_contract_of_the_addressed_node
  - crates/unica-coder/src/infrastructure/daemon/v13_read_modes.rs::operation_dictionary_tests::a_retired_operation_route_follows_roles_and_inherited_content
  - crates/unica-coder/src/domain/apply.rs::a_renamed_operation_target_respects_the_item_role
  - crates/unica-coder/src/domain/operation_contract.rs::role_contracts_publish_the_entry_shape
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::retired_subnode_arguments_route_next_to_the_subnode_address
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/form_resource.rs::right_set_grants_lists_and_restricts_rights_on_the_staged_document
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::an_area_is_written_only_where_it_exists_and_created_only_by_area_add
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::a_missing_rights_entry_or_area_routes_next_to_the_parent
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::a_missing_target_routes_next_to_the_parent_dictionary
  - crates/unica-coder/src/domain/apply.rs::dcs_schema_nodes_and_structure_items_take_their_subject_from_the_address
  - crates/unica-coder/src/domain/apply.rs::a_repeated_schema_name_routes_next_to_the_branch_and_every_match
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/dcs_mxl.rs::a_data_set_is_found_by_name_inside_unions
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/dcs_mxl.rs::a_structure_item_is_found_by_its_named_ancestors
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::retired_dcs_container_arguments_route_next_to_the_structure_item
  - crates/unica-coder/src/infrastructure/v13_read_projection.rs::dcs_structure_tests::repeated_total_paths_stay_rows_and_are_not_chosen
  - crates/unica-coder/src/domain/apply.rs::a_form_item_or_event_named_by_an_argument_is_refused_with_its_address
  - crates/unica-coder/src/domain/apply.rs::a_form_item_role_narrows_element_operations
  - crates/unica-coder/src/domain/operation_contract.rs::form_move_contract_selects_one_source_and_preserves_exact_names
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::honest_can_on_form_items_and_events
  - crates/unica-coder/src/infrastructure/daemon/v13_can_honesty_tests.rs::retired_form_arguments_route_next_to_the_item_or_event_address
  - crates/unica-coder/src/infrastructure/daemon/form_move_tests.rs::column_group_and_its_columns_are_addressable_through_the_wire
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/form_resource.rs::form_add_then_element_and_event_edits_compose_in_one_batch
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/form_resource.rs::a_form_edit_writes_only_the_addressed_item
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
`structure.set` и `structure.patch` (`values.variant`), дочерняя подсистема
у `childSubsystem.remove` (`items[].name`, элемент-строка `items[]`,
`values.name`) и метод у `code.replace` (`selector.method`). Если из значения
строится адрес, где операция исполнима, вызов получает `bad_value` до
планирования, даже когда операция исполнима и на адресе вызова. Первый
`next` ведёт к узлу, который называло значение, второй — к ветви узлов
этого вида. Если значение повторяло адрес вызова, отказ говорит снять
аргумент. Иначе
снятый ключ отвергают контракт или разбор планировщиком, тоже `bad_value`.
Набор, названный снятым `dataSet`, — это сегмент адреса вызова с тем же
именем, иначе набор, найденный по имени в схеме макета (в том числе член
объединения), иначе набор верхнего уровня. Если наборов с этим именем
в схеме несколько, адрес за вызывающего не выбирается: первый `next` ведёт
к ветви наборов `Template.<макет>.DataSet`, следующие — к `can` каждого
совпавшего узла. Операции запроса исполнимы
только на запросе набора `DataSet.<набор>.Query`; вызов на самом наборе
получает отказ «не тот узел» с `next` к `can` его запроса. Прежний адрес
`Template.<макет>.Query.<набор>` отвергается с `next` к запросу этого
набора, а для операций набора — к самому набору; остальные операции набора
на узле запроса не исполнимы.
Дочерняя подсистема исключается из родителя только по своему адресу.
Текстовый якорь `selector.anchor` у `code.replace` и `code.insert` остаётся
аргументом.

Операции подузлов исполняются по адресу подузла, а не владельца: запись
прав `right.set` — на `Role.<роль>.Right.<Вид>_<Имя>`, ячейки `mxl.set` —
на области `Template.<макет>.Area.<область>`, `variant.set` и
`variant.remove` — на `Setting.<вариант>`, `parameter.set` и
`parameter.rename` — на `Parameter.<параметр>`, `field.set` и
`fieldRole.set` — на `DataSet.<набор>.Field.<поле>`, вставка `code.insert`
до или после метода — на `Method.<метод>` с `position`. В схеме компоновки
`dataSource.set` и `dataSource.remove` исполняются на
`Template.<макет>.DataSource.<источник>`, `calculatedField.set` и
`calculatedField.remove` — на `CalculatedField.<путь>`, `total.set`
и `total.remove` — на `Total.<путь>`. Отбор, выбор, порядок, условное
оформление, параметры вывода и `structure.add` берут контейнер настроек
из адреса: вариант `Setting.<вариант>` или именованный элемент его
структуры `Setting.<вариант>.Item.<имя>[.Item.<имя>…]`; `structure.patch`
и `structure.remove` исполняются на самом элементе, а `structure.set`,
заменяющая всю структуру, и параметры данных — только на варианте;
у параметров данных ключа `group` больше нет, и контракт отвергает его как
неизвестный.
Безымянный элемент структуры адреса не имеет и прозрачен: его потомки
адресуются от ближайшего именованного предка, а имя адресует элемент,
только если оно единственно на своём уровне. Итог с повторённым
`dataPath` (итоги одного поля по разным группировкам) адреса не получает
и остаётся строкой ветви `Total`. Прежний аргумент
(`values.object`, `values.area`, `values.name`, `values.field`,
`values.dataPath`, `selector.method`, а у операций контейнера настроек
`group` и `parent`) получает тот же отказ `bad_value`
с `next` к `can` подузла и к ветви его вида; элемент структуры, названный
прежним аргументом, ищется в структуре варианта, а повторённое имя ведёт
к ветви `Setting.<вариант>.Item` и к `can` совпавших элементов. Ключи
строк настроек без устойчивой идентичности — поле отбора, индекс выбора,
порядка и оформления, составной ключ связи наборов — остаются аргументами.
Точка в имени подузла
экранируется `\.`: поле с составным `dataPath` адресуется
`Field.Owner\.Code`, запись прав вложенного объекта роли
(`Kind.Name.Command.C`, реквизит) — `Right.Kind_Name\.Command\.C`,
область с точкой в имени — `Area.<имя с \.>`. Запись прав адресуется только
каноническим именем `<Вид>_<Имя>`, а не коротким псевдонимом чтения.

Узел создаёт только операция родителя с именем в аргументах, а `*.set`
и `*.remove` пишут в существующий узел. Запись прав создаёт `right.add`
на роли с начальным набором прав, область — `area.add` на макете
с начальными ячейками, форму — `form.add` на владельце. `right.set` и
`mxl.set` на отсутствующем узле отвечают `not_found` с `next` к родителю,
в чьём `can` есть создающая операция; опечатка в адресе узла не создаёт.
`right.add` и `area.add` с именем существующего узла отвергаются
`bad_value` и называют адрес для `*.set`. Отказ по снятому аргументу
у `right.set` и `mxl.set` называет создающую операцию родителя, а третий
`next` ведёт к её контракту `<родитель>.Can.<операция>`. Контракты `right.add`
и `right.set` публикуются по адресу операции, как у других семейств. Снятая
операция `form.create` в реестре отсутствует: вызов получает `bad_value`
с `next` к владельцу формы, где исполнима `form.add`. Псевдонимов
прежних форм вызова нет.

Элемент и событие формы адресуются цепочкой
`Form.<форма>[.Item.<элемент>[.Item.<вложенный>…]][.Event.<событие>]`, как их
показывает `view`; автоматическая командная панель формы и таблицы видна
элементом `Item`. Группа колонок таблицы (`ColumnGroup`) и её колонки тоже
видны в `view` и адресуются полной цепочкой `Item`. `item.add`
добавляет в форму или в элемент-контейнер,
названный адресом, `item.remove` удаляет элемент по его адресу, `item.move`
переносит элемент, названный полным текущим адресом, в допустимый контейнер
или в корень формы,
`event.bind` привязывает обработчик (`values.handler`, `values.callType`)
на адресе события. Реквизит и команду формы добавляют `attribute.add`
и `command.add` на адресе формы. Операции формы названы по виду своего
узла (`Item`, `Attribute`, `Command`), и имя реквизита и команды общее
с объектом метаданных: запись реестра и контракт `<узел>.Can.<операция>`
выбирает адрес вызова — на форме это реквизит и команда формы, на объекте
это члены его описания. Отказ «не тот узел» у общего имени ведёт `next`
к самому глубокому узлу, где исполнима запись любого из семейств.
Снятые `element.add`, `element.remove`, `formAttribute.add`
и `formCommand.add` в реестре отсутствуют. Вызов получает `bad_value`
без записи, а `next` ведёт к контракту заменившей операции формы
на ближайшем узле, где она исполнима: на адресе вызова или на его предке,
как отказ «не тот узел», с учётом роли элемента. Удаление узла, которое
его роль не принимает (автоматическая командная панель), другим узлом
не подменяется. К члену объекта метаданных снятая операция формы
не ведёт. Если такого узла в цепочке адреса нет, отказ называет форму
адреса заменившей операции и словарь узла. Чтение `<узел>.Can.<снятая>`
отвечает `bad_value` с `next` к контракту заменившей, у `form.create` —
к `form.add` владельца; у заимствованного узла с содержимым родителя
`next` ведёт к словарю узла.
Словарь элемента учитывает его роль: в поле, декорацию
и кнопку элемент не добавляется, автоматическая командная панель отдельно
от владельца не удаляется и не переносится. Позиция `after` остаётся именем
соседа в том же контейнере; сосед из другого контейнера отвергается
`bad_value`. У `item.move` поле `values.into` обязательно: строка называет
контейнер, явный `null` — корень формы. Снятые
аргументы — `items[].into` у `item.add`, `items[].name`, элемент-строка
`items[]` и `values.name` у `item.remove`, `values.element`
и `values.event` у `event.bind` — получают тот же отказ `bad_value`
с `next` к узлу, который называло значение: к элементу по его пути
в форме, к событию с именем из реестра. Элемент, которого в форме нет,
даёт `not_found` без записи. `event.bind` принимает каждое событие,
опубликованное узлом `Event`: проверка привязки допускает события
каталога платформы, по которому `view` строит эти узлы.

Проверки проходят настоящий вызов `view` для объекта метаданных и
отдельно проверяют проекции корня, модуля, неизвестного вида и операции
без реализации. Двусторонняя сверка обходит проекции чтения фикстур
приёмочного корпуса и для представительных узлов каждой пары «маршрут,
вид» сравнивает словарь с предварительной проверкой, разбором семейства
в обход реестра (расхождение допустимо только как названное намеренное
сужение), фильтром формата макета в планировщике и файлами плана.

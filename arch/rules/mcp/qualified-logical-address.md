---
id: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
check:
  - crates/unica-coder/src/domain/address.rs::qualified_addresses_are_table_driven_canonical_and_arbitrarily_deep
  - crates/unica-coder/src/domain/address.rs::qualified_addresses_reject_unqualified_malformed_and_noncanonical_roots
  - crates/unica-coder/src/domain/address.rs::metadata_aliases_reuse_v12_evidence_while_structural_aliases_stay_separate
  - crates/unica-coder/src/domain/address.rs::unqualified_input_resolves_only_with_one_source_set_and_stays_qualified
  - crates/unica-coder/src/domain/address.rs::configuration_kind_is_rejected_everywhere_except_the_sole_root
  - crates/unica-coder/src/domain/address.rs::nameless_kinds_do_not_consume_the_next_segment_as_a_name
  - crates/unica-coder/src/domain/address.rs::a_named_kind_still_takes_the_segment_that_follows_it
  - crates/unica-coder/src/domain/address.rs::neither_a_configuration_prefix_nor_an_invented_interface_name_is_accepted
  - crates/unica-coder/src/domain/address.rs::a_can_segment_takes_the_rest_of_the_address_as_the_operation_name
  - crates/unica-coder/src/domain/address.rs::an_escaped_dot_stays_inside_one_name_and_round_trips
  - crates/unica-coder/src/domain/address.rs::an_escaped_dot_is_a_name_only_on_document_routes
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::names_with_a_dot_are_published_escaped_and_read_back
  - crates/unica-coder/src/domain/apply.rs::retired_names_with_a_dot_route_to_escaped_addresses
  - crates/unica-coder/src/infrastructure/v13_read_projection.rs::dcs_structure_tests::setting_structure_rows_never_fabricate_or_choose_named_addresses
---

# Логический адрес называет набор исходников и путь по видам объектов

Канонический адрес имеет вид `<набор>:<Вид>.<Имя>...` и всегда содержит
непустое имя набора исходников. Виды и прикладные имена могут чередоваться
на произвольной глубине. Русские псевдонимы видов приводятся к принятым
английским именам; написание прикладных имён сохраняется. Физического пути
в адресе нет.

Точка внутри прикладного имени пишется `\.`, остальные точки разделяют
сегменты: поле схемы с составным `dataPath` — `…Field.Контрагент\.ИНН`,
группа интерфейса — `…Group.CommandGroup\.Цены`. Других
последовательностей нет: `\` не перед точкой отклоняется. Точку несут
только имена из данных документов, и решает маршрут, а не вид сегмента:
поле, параметр и область внутри макета, вычисляемое поле и итог схемы,
элемент структуры варианта, группа командного интерфейса,
тип и свойство пакета XDTO, запись права роли. Имя объекта метаданных
и его члена точки не содержит, и экранированная точка в нём отклоняется
при разборе — в том числе у параметра формы или операции веб-сервиса.
Имя без точки пишется как прежде. Имя элемента структуры варианта СКД
(`…Setting.<вариант>.Item.<имя>`) — свободный текст настроек, а не
идентификатор: пробелы, скобки и другие знаки в нём допустимы, кроме
обратной косой черты; точка экранируется так же. В остальных местах адреса
такое имя отклоняется. Напечатанный адрес разбирается в тот же
адрес; адреса, которые публикуют чтение и подсказки `next`, экранируют имя
так же.

Вид в конце без имени обозначает ветвь, например
`main:Catalog.Валюты.Attribute`. У `Configuration` и `Interface` имени
не бывает: следующий сегмент разбирается как вид. Выдуманное имя
отклоняется с объяснением этой причины. `Configuration` разрешён только
как самостоятельный корень `main:Configuration` и перед вопросом к его
словарю `main:Configuration.Can.<op>`.

Сегмент `Can` спрашивает операцию словаря узла и всегда последний: его имя —
весь остаток адреса вместе с точками (`main:Catalog.Валюты.Can.query.set`),
потому что имя операции само содержит точку. Узел вопроса — адрес до `Can`.
После ветви без имени (`...Attribute`, `...Body`) `Can`, за которым идёт
не вид, а имя операции, — тот же вопрос, а не имя ветви; `Can`, за которым
идёт вид, остаётся прикладным именем. `main:Can.<op>` канонически пишется
`main:Configuration.Can.<op>`. Такой адрес называет операцию, а не узел
исходников: его читает только `unica.view`, другие инструменты отклоняют
его по [правилу словаря операций](operation-dictionary.md).

Разрешение входного адреса в контексте проекта может подставить пропущенный
набор, только если доступен ровно один. Результат всё равно содержит набор;
строгий разбор канонического адреса его не подставляет.

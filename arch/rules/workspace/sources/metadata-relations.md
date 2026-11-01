---
id: INV.SOURCE.RELATIONS-ANSWER-IN-ONE-BRANCH
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests/journal_column_references.rs::journal_column_references_keep_five_distinct_targets_and_read_the_returned_field
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata.rs::every_reference_of_an_object_answers_in_one_relation_branch
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::a_subsystem_shows_its_content_and_reads_registered_children_at_depth
  - crates/unica-metadata-integration-tests/tests/subsystem_topology.rs::memberships_name_the_listing_subsystem_its_ancestors_and_standard_library_roots
  - crates/unica-coder/src/infrastructure/v13_large_configuration.rs::retained_subsystem_topology_is_reused_until_a_descriptor_changes
---

# Ссылки объекта собраны в одной ветви Relation

Ссылки объекта метаданных на другие объекты возвращаются в ветви `Relation`.
Каждый элемент содержит имя связи `relation`, логический адрес цели `at`
и её вид `kind`. Владельцы, движения, основания и остальные виды ссылок
различаются именем связи, а не отдельными ветвями. Состав подсистемы —
связь `content` той же ветви.

Ветвь называет и подсистемы, в состав которых входит объект: связь
`subsystem` с полем `direct` — `true` у подсистемы, которая перечисляет
объект в своём составе, `false` у её родителей. Связь `library` называет
корень библиотеки объекта с источником признака `standardLibraryModule`.
Корень библиотеки — подсистема верхнего уровня вне командного интерфейса,
в дереве которой лежит общий модуль `ОбновлениеИнформационнойБазы…`. Это
соглашение стандартов разработки 1С, а не свойство платформы: без такого
модуля корней нет. Объект из двух библиотек называет обе. У самой
подсистемы-корня признак `libraryRoot` с тем же источником.

Цель читается по собственному адресу. Спуск внутрь элемента `Relation`
завершается `not_found`. Класс объектов и пространство имён без адреса
конкретного объекта в эту ветвь не входят.

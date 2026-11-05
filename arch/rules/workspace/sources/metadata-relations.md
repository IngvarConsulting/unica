---
id: INV.SOURCE.RELATIONS-ANSWER-IN-ONE-BRANCH
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests/subscription_handler.rs::subscription_relation_preserves_sources_and_adds_one_method_link
  - crates/unica-coder/src/infrastructure/v13_read/tests/subscription_handler.rs::malformed_handlers_do_not_invent_links_and_missing_methods_refuse_on_follow
  - crates/unica-coder/src/infrastructure/v13_read/tests/subscription_handler.rs::damaged_handler_xml_keeps_source_and_observation_limits
  - crates/unica-coder/src/infrastructure/v13_read/tests/metadata.rs::every_reference_of_an_object_answers_in_one_relation_branch
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::a_subsystem_shows_its_content_and_reads_registered_children_at_depth
  - crates/unica-metadata-integration-tests/tests/subsystem_topology.rs::memberships_name_the_listing_subsystem_its_ancestors_and_standard_library_roots
  - crates/unica-coder/src/infrastructure/v13_large_configuration.rs::retained_subsystem_topology_is_reused_until_a_descriptor_changes
---

# Ссылки объекта собраны в одной ветви Relation

Ссылки объекта метаданных на другие объекты и методы возвращаются в ветви `Relation`.
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

Подписка на событие возвращает единственную связь `handler` на `Method`
общего модуля в своём наборе исходников. Значение `Handler` и связи `source`
сохраняются. Пустая или повреждённая ссылка адреса не порождает; синтаксически
корректная ссылка может вести к отсутствующему методу — его чтение отвечает
`not_found`. Навигация не подтверждает корректность привязки: совместимость
источников, события и обработчика проверяет [валидатор подписки](event-subscription-handler.md).

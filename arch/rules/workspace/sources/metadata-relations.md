---
id: INV.SOURCE.RELATIONS-ANSWER-IN-ONE-BRANCH
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::every_reference_of_an_object_answers_in_one_relation_branch
---

# Ссылки объекта собраны в одной ветви Relation

Ссылки объекта метаданных на другие объекты возвращаются в ветви `Relation`.
Каждый элемент содержит имя связи `relation`, логический адрес цели `at`
и её вид `kind`. Владельцы, движения, основания и остальные виды ссылок
различаются именем связи, а не отдельными ветвями.

Цель читается по собственному адресу. Спуск внутрь элемента `Relation`
завершается `not_found`. Класс объектов и пространство имён без адреса
конкретного объекта в эту ветвь не входят.

---
id: INV.SOURCE.RELATIONS-ANSWER-IN-ONE-BRANCH
status: active
governs: product
decision: DEC.2026-09-10.ONE-RELATION-BRANCH-FOR-EVERY-REFERENCE
check: crates/unica-coder/src/infrastructure/v13_read/tests.rs::every_reference_of_an_object_answers_in_one_relation_branch
scope: [product, source]
---

# Ссылки объекта живут в одной ветви и показывают наружу

Всякая ссылка объекта метаданных на другой объект приходит элементом ветви
`Relation` с полем `relation`, адресом цели и её видом. Второй ветви под
ссылки не заводится, а спуск в элемент отказывает: цель читается по своему
адресу.

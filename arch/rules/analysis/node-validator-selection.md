---
id: INV.SURFACE.CHECK-INFERS-VALIDATORS
check:
  - crates/unica-coder/src/application/v13/check.rs::every_node_kind_owns_its_validators_without_a_caller_choice
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_catalog_locks_the_eight_domain_contracts_without_publishing_them
  - tests/ci/test_acceptance_scenarios.py::AcceptanceCorpusRunTests.test_every_wire_answers_its_frozen_classes
---

# Вид узла определяет его валидаторы

`unica.check` принимает логический адрес `at` и сам выбирает все валидаторы
по виду и свойствам прочитанного узла. Аргумента для выбора валидатора нет.
Например, вид макета определяет проверку DCS или MXL, а модуль и тело
модуля проверяются анализатором BSL.

Ответ объединяет результаты выбранных проверок: `status`, список
`validators` и диагностики с указанием валидатора. Узел, которому валидатор
не назначен, отвечает читаемостью (`status: "readable"`).

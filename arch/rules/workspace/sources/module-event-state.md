---
id: INV.SOURCE.MODULE-EVENT-STATE
check:
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::form_binding_owners_and_all_four_event_states_are_projected
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::platform_event_state_requires_exact_kind_parameter_shape_and_effective_contexts
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::form_handler_parameters_are_compared_with_the_item_and_the_form_check_rule
  - tests/ci/test_acceptance_scenarios.py::AcceptanceCorpusRunTests.test_every_wire_answers_its_frozen_classes
gap: https://github.com/IngvarConsulting/unica/issues/973
---

# Состояние события отличает отсутствие обработчика от неверной привязки

`available` означает применимое событие без необязательного обработчика.
`implemented` требует совместимый метод и корректную привязку.
`missing` означает, что привязка называет отсутствующий метод;
`invalid` — что существующий метод или привязка не подходят.

В проверку совместимости входят вид метода, параметры и контексты.
Параметры обработчика события формы, её элемента, таблицы, колонки или команды
сравниваются со списками параметров события по
[правилу проверки формы](form-event-handler-parameters.md). Они не совместимы,
если подходят другому событию того же владельца или требуют больше
обязательных параметров, чем передаёт платформа. Так же сравниваются
обработчик команды и привязка с `callType`, которые сама проверка формы
не проверяет. Прежняя короткая сигнатура и другие имена параметров совместимы.
Сигнатура события элемента, таблицы и колонки показывается
с `Элемент` первым параметром, если его передаёт платформа: например,
`Процедура ПриИзменении(Элемент)`.
Метод формы сохраняет обратные связи с событиями их фактических владельцев.
Проверки проходят построитель проекций событий.

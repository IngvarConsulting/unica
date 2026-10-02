---
id: INV.SOURCE.MODULE-EVENT-STATE
check:
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::form_binding_owners_and_all_four_event_states_are_projected
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::platform_event_state_requires_exact_kind_parameter_shape_and_effective_contexts
gap: https://github.com/IngvarConsulting/unica/issues/973
---

# Состояние события отличает отсутствие обработчика от неверной привязки

`available` означает применимое событие без необязательного обработчика.
`implemented` требует совместимый метод и корректную привязку.
`missing` означает, что привязка называет отсутствующий метод;
`invalid` — что существующий метод или привязка не подходят.

В проверку совместимости входят вид метода, параметры и контексты.
Метод формы сохраняет обратные связи с событиями их фактических владельцев.
Проверки проходят построитель проекций событий.

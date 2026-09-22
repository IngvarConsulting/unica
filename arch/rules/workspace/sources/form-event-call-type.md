---
id: INV.PLATFORM.FORM-EVENT-CALL-TYPE
check:
  - crates/unica-coder/src/infrastructure/bsl_module_projection.rs::form_event_implementation_requires_call_type_valid_for_the_form_definition
---

# Наличие обработчика не делает недопустимую привязку события рабочей

Событие формы получает состояние `implemented` только при допустимом
для этой формы значении XML-атрибута `callType`.

Например, привязка `After` с подходящим обработчиком недопустима у обычной
управляемой формы, но допустима у формы расширения. Неизвестное значение
`Instead` оставляет событие в состоянии `invalid`.

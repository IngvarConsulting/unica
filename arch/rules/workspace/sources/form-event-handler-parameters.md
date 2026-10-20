---
id: INV.SOURCE.FORM-EVENT-HANDLER-PARAMETERS
check:
  - crates/unica-forms-integration-tests/tests/event_registry.rs::item_parameter_follows_the_help_and_the_dump_evidence
  - crates/unica-forms-integration-tests/tests/event_registry.rs::handler_signature_puts_the_item_first_only_where_the_platform_passes_it
  - crates/unica-forms-integration-tests/tests/event_registry.rs::form_kind_selects_one_variant_of_each_event
  - crates/unica-forms-integration-tests/tests/event_registry.rs::foreign_signature_needs_a_strictly_better_unambiguous_match_and_another_count
  - crates/unica-forms-integration-tests/tests/event_registry.rs::a_tie_is_broken_by_the_only_leader_with_the_handler_parameter_count
  - crates/unica-forms-integration-tests/tests/event_registry.rs::extra_required_parameters_are_counted_against_every_known_list
  - crates/unica-forms-integration-tests/tests/handler_parameters.rs::legacy_shorter_signatures_give_no_finding
  - crates/unica-forms-integration-tests/tests/handler_parameters.rs::foreign_event_signature_is_a_warning_naming_both_events
  - crates/unica-forms-integration-tests/tests/handler_parameters.rs::same_count_renames_pass_and_table_foreign_signature_is_found
  - crates/unica-forms-integration-tests/tests/handler_parameters.rs::required_parameter_beyond_every_known_list_is_a_warning
  - crates/unica-forms-integration-tests/tests/handler_parameters.rs::object_form_events_follow_the_owner_found_by_the_dump_path
  - crates/unica-forms-integration-tests/tests/handler_parameters.rs::a_table_bound_to_the_dynamic_list_gets_its_extra_events
  - crates/unica-forms-integration-tests/tests/handler_parameters.rs::by_value_names_and_async_handlers_are_compared_by_their_names
  - crates/unica-forms-integration-tests/tests/handler_parameters.rs::extension_forms_compare_only_bindings_without_call_type
  - crates/unica-forms-integration-tests/tests/handler_parameters.rs::missing_or_unreadable_module_and_absent_or_ambiguous_method_give_no_finding
  - tests/ci/test_acceptance_scenarios.py::AcceptanceCorpusRunTests.test_every_wire_answers_its_frozen_classes
---

# Проверка формы находит обработчик с сигнатурой чужого события

`unica.check` формы сравнивает параметры обработчика с известными списками
параметров привязанного события. Обработчик ищется в модуле той же формы.
Списки берутся из [каталога справки](platform-event-catalog.md) `8.3.27`
для вида этой формы: по владельцу в метаданных, главному реквизиту и привязке
таблицы к динамическому списку.
Справка не называет `Элемент`, который платформа передаёт первым параметром
обработчикам событий элемента и таблицы, поэтому проверка его добавляет.
В `8.3.27` действуют исключения:
`ПриПолученииДанныхНаСервере` и `ОбработкаЗаписиНового` таблицы `Элемент` не
получают, а `Нажатие` декорации-надписи получает ещё и `СтандартнаяОбработка`.
Тот же список параметров с `Элемент` использует [состояние события](module-event-state.md)
в `unica.view` и [реализация события](event-implementation-pair.md).

Обработчик с меньшим числом параметров законен: платформа вызывает обработчики
прежних сигнатур и сопоставляет параметры по смыслу.

Проверка выдаёт предупреждение в двух случаях:

- имена параметров обработчика совпадают со списком другого события того же
  владельца. Совпадений с ним не меньше двух и больше, чем с привязанным
  событием. Если столько же совпадений у нескольких событий, выбирается
  единственное из них, у которого столько же параметров, сколько у
  обработчика; иначе находки нет. Число параметров обработчика при этом
  отличается от любого известного списка привязанного события;
- обязательных параметров больше, чем в самом длинном известном списке
  привязанного события.

Предупреждение называет привязанное событие русским именем из справки,
обработчик, его параметры и ожидаемый список параметров, а в первом случае
ещё событие, которому подходят параметры, и его список. Вердикт `passed` сохраняется: платформа
имена параметров не проверяет. Привязки с `callType`, события без записи
в каталоге, отсутствующий или неразбираемый модуль, отсутствующий или
неоднозначный метод не проверяются.

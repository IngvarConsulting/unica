---
id: INV.SOURCE.FORM-EVENT-HANDLER-PARAMETERS
check:
  - crates/unica-coder/src/infrastructure/native_operations/form_event_registry.rs::item_parameter_follows_the_help_and_the_dump_evidence
  - crates/unica-coder/src/infrastructure/native_operations/form_event_registry.rs::foreign_signature_needs_a_unique_strictly_better_match_and_another_count
  - crates/unica-coder/src/infrastructure/native_operations/form_event_registry.rs::extra_required_parameters_are_counted_against_every_known_list
  - crates/unica-coder/src/infrastructure/native_operations/form.rs::legacy_shorter_signatures_give_no_finding
  - crates/unica-coder/src/infrastructure/native_operations/form.rs::foreign_event_signature_is_a_warning_naming_both_events
  - crates/unica-coder/src/infrastructure/native_operations/form.rs::same_count_renames_pass_and_table_foreign_signature_is_found
  - crates/unica-coder/src/infrastructure/native_operations/form.rs::required_parameter_beyond_every_known_list_is_a_warning
  - crates/unica-coder/src/infrastructure/native_operations/form.rs::extension_call_type_bindings_are_not_compared
  - crates/unica-coder/src/infrastructure/native_operations/form.rs::missing_or_unreadable_module_and_absent_or_ambiguous_method_give_no_finding
  - tests/ci/test_acceptance_scenarios.py::AcceptanceCorpusRunTests.test_every_wire_answers_its_frozen_classes
---

# Проверка формы находит обработчик с сигнатурой чужого события

`unica.check` формы сравнивает параметры обработчика с известными списками
параметров привязанного события. Обработчик ищется в модуле той же формы.
Списки берутся из [каталога справки](platform-event-catalog.md) `8.3.27`.
Справка не называет `Элемент`, который платформа передаёт первым параметром
обработчикам событий элемента и таблицы, поэтому проверка его добавляет.
Исключения подтверждены выгрузкой типовой конфигурации на `8.3.27`:
`ПриПолученииДанныхНаСервере` и `ОбработкаЗаписиНового` таблицы `Элемент` не
получают, а `Нажатие` декорации-надписи получает ещё и `СтандартнаяОбработка`.

Обработчик с меньшим числом параметров законен. Платформа вызывает обработчики
прежних сигнатур и сопоставляет параметры по смыслу. В той же выгрузке
2640 обработчиков `НачалоВыбора` объявляют три параметра из четырёх,
1179 обработчиков `ОбработкаВыбора` поля ввода — три из пяти. Последним
остаётся `СтандартнаяОбработка`, и 1423 и 684 из них ей присваивают значение.

Проверка выдаёт предупреждение в двух случаях:

- имена параметров обработчика совпадают со списком другого события того же
  владельца. Совпадений с ним не меньше двух, больше, чем с привязанным
  событием, и ни у какого третьего события их не столько же. Число параметров
  при этом отличается от любого известного списка привязанного события;
- обязательных параметров больше, чем в самом длинном известном списке
  привязанного события.

Предупреждение называет привязанное событие, обработчик, его параметры
и ожидаемый список параметров, а в первом случае ещё событие, которому
подходят параметры, и его список. Вердикт `passed` сохраняется: платформа
имена параметров не проверяет. Привязки с `callType`, события без записи
в каталоге, отсутствующий или неразбираемый модуль, отсутствующий или
неоднозначный метод не проверяются.

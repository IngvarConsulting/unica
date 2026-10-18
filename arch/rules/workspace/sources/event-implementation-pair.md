---
id: INV.SOURCE.EVENT-IMPLEMENTATION-PAIR
check:
  - crates/unica-coder/src/infrastructure/native_operations/event.rs::staged_property_planner_composes_two_events_and_poisoned_batch_publishes_nothing
  - crates/unica-coder/src/infrastructure/native_operations/event.rs::staged_missing_property_keeps_form_byte_exact_and_platform_creates_only_module
  - crates/unica-coder/src/infrastructure/event_projection.rs::implementation_shape_uses_typed_owner_leaf_and_the_selected_catalog_record
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::staged_code_first_body_materializes_a_missing_module_chain
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::staged_code_missing_module_chain_keeps_a_foreign_appearance
  - tests/ci/test_acceptance_scenarios.py::AcceptanceCorpusRunTests.test_every_wire_answers_its_frozen_classes
---

# Реализация события готовит метод и привязку вместе

`event.implement` готовит совместимый метод и необходимую привязку одним
пакетом. Отсутствующий BSL-файл не требует отдельной инициализации.
Если привязка уже называет нужный отсутствующий метод, её XML не переписывается.
Планирование и отказ следующей операции не публикуют частичный результат.
Обработчик события элемента, таблицы или колонки формы получает `Элемент`
первым параметром, если его передаёт платформа
([список параметров](form-event-handler-parameters.md)); обработчик события
самой формы и команды получает сигнатуру справки.

Проверки проходят планировщик событий и создание первой цепочки каталогов
модуля. Публикация использует общий механизм атомарного применения.
Сценарий S319 приёмочного корпуса заимствует общий модуль без каталога,
проверяет отсутствие записи при предпросмотре, применяет вставку и читает
созданный метод через публичную поверхность.

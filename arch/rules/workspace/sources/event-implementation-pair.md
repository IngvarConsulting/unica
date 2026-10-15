---
id: INV.SOURCE.EVENT-IMPLEMENTATION-PAIR
check:
  - crates/unica-coder/src/infrastructure/native_operations/event.rs::staged_property_planner_composes_two_events_and_poisoned_batch_publishes_nothing
  - crates/unica-coder/src/infrastructure/native_operations/event.rs::staged_missing_property_keeps_form_byte_exact_and_platform_creates_only_module
  - crates/unica-coder/src/infrastructure/event_projection.rs::implementation_shape_uses_typed_owner_leaf_and_the_selected_catalog_record
  - tests/ci/test_acceptance_scenarios.py::AcceptanceCorpusRunTests.test_every_wire_answers_its_frozen_classes
gap: https://github.com/IngvarConsulting/unica/issues/955
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

Проверки проходят планировщик событий. Публикация использует общий механизм
атомарного применения; создание отсутствующих файлов на текущем публичном
пути дополнительно проверяется в `gap`.

---
id: INV.SOURCE.EVENT-IMPLEMENTATION-PAIR
check:
  - crates/unica-coder/src/infrastructure/native_operations/event.rs::staged_property_planner_composes_two_events_and_poisoned_batch_publishes_nothing
  - crates/unica-coder/src/infrastructure/native_operations/event.rs::staged_missing_property_keeps_form_byte_exact_and_platform_creates_only_module
gap: https://github.com/IngvarConsulting/unica/issues/955
---

# Реализация события готовит метод и привязку вместе

`event.implement` готовит совместимый метод и необходимую привязку одним
пакетом. Отсутствующий BSL-файл не требует отдельной инициализации.
Если привязка уже называет нужный отсутствующий метод, её XML не переписывается.
Планирование и отказ следующей операции не публикуют частичный результат.

Проверки проходят планировщик событий. Публикация использует общий механизм
атомарного применения; создание отсутствующих файлов на текущем публичном
пути дополнительно проверяется в `gap`.

---
id: INV.SOURCE.EVENT-DEFINED-TYPE
check:
  - crates/unica-coder/src/infrastructure/metadata_operations.rs::defined_type_event_source_expands_to_concrete_event_classes_and_dependencies
  - crates/unica-coder/src/infrastructure/metadata_operations.rs::defined_type_event_source_cycle_is_rejected_before_publication
  - crates/unica-coder/src/infrastructure/metadata_operations.rs::meta_info_surfaces_a_strict_defined_type_member_parse_failure
---

# Определяемый тип раскрывается перед проверкой подписки

Определяемый тип в источнике подписки раскрывается до составляющих классов
событий, включая вложенные определяемые типы. Цикл или неподходящий член
даёт ошибку: его нельзя молча исключить и проверить лишь удобную часть типа.

Проверки относятся к внутренней подготовке и валидации метаданных.
Использованные описания входят в зависимости; их сохранность перед записью
задаёт [проверка исходных данных](native-preimage-coverage.md).

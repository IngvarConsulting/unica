---
id: INV.SOURCE.EVENT-DEFINED-TYPE
check:
  - crates/unica-coder/src/infrastructure/daemon/metadata_can_tests.rs::canonical_apply_expands_defined_type_event_sources_before_publication
  - crates/unica-coder/src/infrastructure/daemon/metadata_can_tests.rs::canonical_check_expands_nested_defined_type_event_sources
---

# Определяемый тип раскрывается перед проверкой подписки

Определяемый тип в источнике подписки раскрывается до составляющих классов
событий, включая вложенные определяемые типы. Цикл или неподходящий член
даёт ошибку: его нельзя молча исключить и проверить лишь удобную часть типа.

Раскрытие проверяют канонические `unica.apply` и `unica.check`. План
изменённой подписки отказывает при цикле, неподходящем члене и классе,
несовместимом с событием, до публикации.
Использованные описания входят в зависимости; их сохранность перед записью
задаёт [проверка исходных данных](native-preimage-coverage.md).

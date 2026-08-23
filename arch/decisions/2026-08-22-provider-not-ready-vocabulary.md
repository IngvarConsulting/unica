---
id: DEC.2026-08-22.PROVIDER-NOT-READY-VOCABULARY
status: active
governs: product
realized: crates/unica-coder/src/domain/diagnostics.rs::diagnostics_and_search_publish_one_retryable_not_ready_vocabulary
changes: [CTR.WIRE.TOOL-SURFACE]
establishes: [INV.WIRE.PROVIDER-NOT-READY-VOCABULARY]
design: docs/design/2026-08-22-provider-not-ready-contract-design.md
---

# Неготовность поставщика имеет общий словарь

**Решение.** `unica.code.search` и `unica.code.diagnostics` выражают
переходную неготовность поставщика общими типизированными полями. Срок повтора
публикуется только когда его сообщил поставщик; diagnostics дополнительно
указывает `nextAction=status` и не маскирует переходный исход под успех.

Готовый путь не получает нового ожидания, а терминальное состояние поставщика
не становится повторяемым без отдельного свидетельства восстановления.

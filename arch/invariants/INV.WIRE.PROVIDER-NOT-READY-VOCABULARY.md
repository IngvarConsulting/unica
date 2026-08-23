---
id: INV.WIRE.PROVIDER-NOT-READY-VOCABULARY
status: active
governs: product
decision: DEC.2026-08-22.PROVIDER-NOT-READY-VOCABULARY
check: crates/unica-coder/src/domain/diagnostics.rs::diagnostics_and_search_publish_one_retryable_not_ready_vocabulary
scope: [wire]
---

# Search и diagnostics разделяют словарь неготовности

Переходная неготовность в search и diagnostics совпадает по полям `code`,
`retryable`, `detailCode`, `retryAfterMs` и `state`; diagnostics добавляет
`nextAction=status`.

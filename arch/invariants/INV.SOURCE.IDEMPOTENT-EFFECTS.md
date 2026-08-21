---
id: INV.SOURCE.IDEMPOTENT-EFFECTS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::mutating_native_noop_does_not_emit_cache_events
scope: [source]
---

# Семантический noop не публикует событие кеша

Нативная мутация с пустым набором изменений не публикует доменное событие и не
инвалидирует кеш.

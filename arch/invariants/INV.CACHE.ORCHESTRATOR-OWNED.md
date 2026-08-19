---
id: INV.CACHE.ORCHESTRATOR-OWNED
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::runtime_event_is_not_emitted_for_non_invalidating_operations
scope: [cache]
---

# Состоянием рабочего пространства владеет оркестратор

Кеш и состояние принадлежат оркестратору, а не адаптерам. Изменяющая операция порождает
типизированное доменное событие, и применённое изменение запоминает инвалидированный им кеш.

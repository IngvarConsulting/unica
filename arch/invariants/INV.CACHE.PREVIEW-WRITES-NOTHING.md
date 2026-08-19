---
id: INV.CACHE.PREVIEW-WRITES-NOTHING
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/workspace_state.rs
scope: [cache]
---

# Предпросмотр не оставляет следов

Сухой прогон сообщает о последствиях и не пишет ни в источник, ни в кеш.

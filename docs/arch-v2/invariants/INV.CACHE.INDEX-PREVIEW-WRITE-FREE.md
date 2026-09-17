---
id: INV.CACHE.INDEX-PREVIEW-WRITE-FREE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/workspace_index.rs::dry_run_does_not_start_indexing_or_write_state
scope: [cache]
---

# Предпросмотр индекса не оставляет состояния

Сухой прогон индексирования не запускает построитель и не пишет состояние
индекса рабочего пространства.

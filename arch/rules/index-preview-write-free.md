---
id: INV.CACHE.INDEX-PREVIEW-WRITE-FREE
check:
  - crates/unica-coder/src/infrastructure/workspace_index.rs::dry_run_does_not_start_indexing_or_write_state
---

# Предпросмотр индекса не оставляет состояния

Preview индексирования не запускает построитель и не записывает состояние
индекса рабочего пространства.

---
id: INV.CACHE.INDEX-PREVIEW-WRITE-FREE
check:
  - crates/unica-coder/src/infrastructure/workspace_index.rs::dry_run_does_not_start_indexing_or_write_state
---

# Предпросмотр индексирования не запускает работу

При предварительном просмотре индексирования (`preview`) Unica не начинает
строить поисковый индекс проекта и не записывает файл его состояния.

Тест проверяет, что команда построения индекса не запускалась и файл
состояния не появился.

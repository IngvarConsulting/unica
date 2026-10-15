---
id: INV.CACHE.INDEX-PREVIEW-WRITE-FREE
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::public_metadata_apply_preview_is_index_write_free_and_matches_real_plan
---

# Предпросмотр индексирования не запускает работу

При предварительном просмотре индексирования (`preview`) Unica не начинает
строить поисковый индекс проекта и не записывает файл его состояния.

Тест проверяет, что команда построения индекса не запускалась и файл
состояния не появился.

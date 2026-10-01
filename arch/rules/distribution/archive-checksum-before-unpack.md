---
id: INV.PKG.CORRUPT-ARCHIVE-NOT-READY
check:
  - crates/unica-bootstrap/tests/runtime_install.rs::corrupt_archive_never_publishes_a_ready_runtime
---

# Архив с неверной контрольной суммой не распаковывается

Установщик сравнивает SHA-256 загруженного архива с суммой из манифеста.
При несовпадении он отклоняет архив до распаковки и не создаёт маркер
готовности `.ready.json` в кеше установки.

---
id: INV.PKG.CONCURRENT-INSTALL-ONCE
check:
  - crates/unica-bootstrap/tests/runtime_install.rs::concurrent_installers_download_and_publish_once
  - crates/unica-bootstrap/tests/runtime_install.rs::two_sessions_acquire_one_engine_once
---

# Параллельные запросы используют одну установку

Параллельные вызовы установки одной и той же поставки в общем кеше получают
один готовый каталог. Архив загружается один раз; остальные вызовы используют
результат завершённой установки.

Проверки ядра и движка используют по два вызова в отдельных потоках одного
процесса.

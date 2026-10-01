---
id: INV.PKG.VERIFIED-ATOMIC-INSTALL
check:
  - crates/unica-bootstrap/tests/runtime_install.rs::valid_archive_is_published_with_a_ready_marker
  - crates/unica-bootstrap/tests/runtime_install.rs::ready_marker_waits_for_the_complete_runtime_file_closure
  - crates/unica-bootstrap/tests/runtime_install.rs::install_closure_rejects_unsafe_or_drifted_archives_without_ready
  - crates/unica-bootstrap/tests/runtime_install.rs::traversal_archive_is_rejected_before_publication
---

# Установка становится готовой после проверки всех файлов

При новой распаковке установщик проверяет полный набор файлов по манифесту:
пути должны совпадать, каждый файл должен быть обычным файлом с указанной
SHA-256. Пропущенный, лишний, повторяющийся или изменённый файл, ссылка либо
путь за пределы временного каталога приводят к отказу.

Только полностью проверенный набор публикуется как готовая установка
с маркером `.ready.json`. Ошибка проверки не оставляет готовую установку.

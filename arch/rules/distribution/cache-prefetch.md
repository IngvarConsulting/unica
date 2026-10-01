---
id: INV.PKG.CACHE-PREFETCH
check:
  - crates/unica-bootstrap/tests/runtime_install.rs::prefetch_delivers_every_artifact_the_target_needs
  - crates/unica-bootstrap/tests/runtime_install.rs::a_warm_cache_makes_prefetch_download_nothing
  - crates/unica-bootstrap/tests/runtime_install.rs::an_invalid_ready_marker_makes_prefetch_report_a_download
---

# Предварительный прогрев доставляет все артефакты выбранной платформы

`prefetch` устанавливает все артефакты выбранной целевой платформы,
перечисленные в манифесте, включая ядро и движки. Результат называет
каждый артефакт и сообщает, потребовалась ли загрузка.

Повторный прогрев использует готовые проверенные установки без загрузки.
Некорректная отметка готовности требует повторной установки, и результат
прогрева сообщает об этой загрузке.

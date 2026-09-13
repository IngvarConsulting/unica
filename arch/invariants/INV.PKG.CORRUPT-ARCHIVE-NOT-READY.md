---
id: INV.PKG.CORRUPT-ARCHIVE-NOT-READY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-bootstrap/tests/runtime_install.rs::corrupt_archive_never_publishes_a_ready_runtime
scope: [pkg]
---

# Повреждённый архив не становится готовой установкой

Архив с SHA-256, отличным от манифеста, отклоняется до распаковки и не получает
маркер готовности в кеше runtime.

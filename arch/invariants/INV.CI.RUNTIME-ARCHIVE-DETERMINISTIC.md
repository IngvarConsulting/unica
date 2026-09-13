---
id: INV.CI.RUNTIME-ARCHIVE-DETERMINISTIC
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_runtime.py::test_runtime_archive_is_deterministic_and_target_only
scope: [ci, pkg]
---

# Архив ядра детерминирован и ограничен своей целью

Повторная упаковка даёт те же байты, архив содержит только файлы выбранной цели
и несёт нормализованные метаданные элементов.

---
id: INV.PKG.BUILD-TOOL-CLOSURE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_build_unica_tools.py::test_bundle_builder_downloads_shared_archive_once_and_declares_runtime_closure
scope: [pkg, product]
---

# Сборщик объявляет полное содержимое внешнего инструмента

Общий архив внешнего инструмента загружается один раз, а результат сборки
перечисляет замкнутый набор его файлов с проверяемыми метаданными.

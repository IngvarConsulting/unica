---
id: INV.PKG.RUNTIME-TOOL-CLOSURE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_runtime.py::test_runtime_packager_rejects_missing_extra_and_metadata_drift
scope: [pkg, product]
---

# Пакет runtime совпадает с объявленным набором файлов

Упаковщик runtime отвергает отсутствующий или лишний файл, а также расхождение
контрольной суммы или размера с метаданными сборки.

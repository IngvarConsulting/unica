---
id: INV.CI.RUNTIME-METADATA-HASHES
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_runtime.py::test_metadata_hashes_archive_and_each_runtime_file
scope: [ci, pkg]
---

# Метаданные связывают архив с каждым файлом runtime

Описание runtime содержит контрольную сумму архива и контрольные суммы,
размеры и режимы всех файлов внутри него.

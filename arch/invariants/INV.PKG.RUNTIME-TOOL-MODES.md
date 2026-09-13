---
id: INV.PKG.RUNTIME-TOOL-MODES
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_runtime.py::test_runtime_packager_rejects_mode_drift
scope: [pkg, product]
---

# Режимы файлов runtime совпадают с манифестом

Упаковщик отвергает файл, чья исполняемость расходится с объявленным режимом.

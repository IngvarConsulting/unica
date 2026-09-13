---
id: INV.PKG.RUNTIME-TOOL-PATHS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_runtime.py::test_runtime_packager_rejects_duplicate_and_out_of_closure_tool_paths
scope: [pkg, product]
---

# Пути runtime уникальны и принадлежат объявленному набору

Упаковщик отвергает повтор пути и файл инструмента вне его замкнутого набора.

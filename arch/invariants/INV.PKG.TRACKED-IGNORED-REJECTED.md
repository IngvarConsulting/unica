---
id: INV.PKG.TRACKED-IGNORED-REJECTED
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_plugin.py::test_plugin_source_copy_rejects_tracked_nested_ignored_dir
scope: [pkg, product]
---

# Отслеживаемый файл в игнорируемом дереве не маскируется

Упаковщик отвергает отслеживаемый файл внутри вложенного генерируемого дерева,
даже если путь одновременно совпадает с правилом игнорирования.

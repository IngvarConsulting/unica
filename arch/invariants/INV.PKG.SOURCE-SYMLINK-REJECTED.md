---
id: INV.PKG.SOURCE-SYMLINK-REJECTED
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_plugin.py::test_plugin_source_copy_rejects_tracked_symlink
scope: [pkg, product]
---

# Символическая ссылка не становится содержимым плагина

Копирование отслеживаемого исходного дерева прекращается при встрече
символической ссылки и не переносит её цель в пакет.

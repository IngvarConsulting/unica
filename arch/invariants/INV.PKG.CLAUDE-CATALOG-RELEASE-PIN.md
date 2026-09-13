---
id: INV.PKG.CLAUDE-CATALOG-RELEASE-PIN
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_plugin.py::test_claude_catalog_pins_the_release_tag
scope: [pkg, product]
---

# Каталог Claude адресует подкаталог закреплённого выпуска

Сгенерированная запись каталога Claude использует источник `git-subdir`, путь
`plugins/unica` и переданный неизменяемый тег выпуска.

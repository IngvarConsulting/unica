---
id: INV.PKG.TRACKED-BIN-REJECTED
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_plugin.py::test_plugin_source_copy_rejects_tracked_source_bin
scope: [pkg, product]
---

# Отслеживаемый каталог бинарников не входит в пакет

Упаковщик прекращает работу, если под исходным корнем плагина появился
отслеживаемый файл в генерируемом каталоге `bin`.

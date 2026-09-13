---
id: INV.PKG.CODEX-CATALOG-RELEASE-PIN
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_plugin.py::test_generated_marketplace_is_thin_pinned_and_target_neutral
scope: [pkg, product]
---

# Каталог Codex адресует подкаталог закреплённого выпуска

Сгенерированная запись каталога Codex использует источник `git-subdir`, путь
`plugins/unica` и тег текущего выпуска.

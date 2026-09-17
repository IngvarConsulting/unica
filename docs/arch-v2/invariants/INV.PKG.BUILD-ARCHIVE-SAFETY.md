---
id: INV.PKG.BUILD-ARCHIVE-SAFETY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_build_unica_tools.py::test_verified_archive_rejects_unsafe_and_drifted_members
scope: [pkg, product]
---

# Внешний архив извлекается только как проверенные обычные файлы

Сборщик отвергает небезопасный путь, ссылку и расхождение состава внешнего
архива до материализации готового набора инструмента.

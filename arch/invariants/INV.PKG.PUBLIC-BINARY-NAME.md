---
id: INV.PKG.PUBLIC-BINARY-NAME
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_version_contract.py::test_public_runtime_binary_name_is_unica
scope: [pkg, product]
---

# Публичное ядро собирается как unica

Cargo-пакет объявляет бинарник `unica`, а запись `unica` в `tools.lock.json`
связывает то же имя с пакетом `unica-coder` и целевым файлом runtime.

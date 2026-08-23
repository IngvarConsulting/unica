---
id: INV.PKG.VERSION-LOCKSTEP
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_version_contract.py::test_every_contract_location_declares_the_same_version
scope: [pkg, product]
---

# Версия поставки едина во всех контрактных местах

Cargo workspace, оба host-манифеста и запись `unica` в `tools.lock.json`
объявляют одну допустимую версию выпуска.

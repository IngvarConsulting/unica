---
id: INV.CI.LOCKED-WORKSPACE-BUILD
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_build_unica_tools.py::test_workspace_binaries_share_one_locked_cargo_build
scope: [ci, pkg]
---

# Ядро и bootstrap собираются одним закреплённым вызовом

Сборщик вызывает `cargo build --release --locked` один раз для пакетов и
бинарников `unica` и `unica-bootstrap` с отдельным целевым каталогом.

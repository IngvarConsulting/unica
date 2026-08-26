---
id: INV.PKG.HOST-MANIFEST-LOCKSTEP
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_plugin.py::test_source_plugin_carries_a_manifest_for_every_host
scope: [host, pkg, product]
---

# Один каталог плагина несёт согласованные host-манифесты

Исходный каталог содержит манифесты Codex и Claude Code, и оба объявляют одну
версию плагина.

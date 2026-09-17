---
id: INV.PKG.TWO-HOSTS-ONE-TREE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_plugin.py::test_packaged_alias_resolves_the_plugin_root_for_both_hosts
scope: [pkg]
---

# Два хоста разрешают один корень плагина

Упакованный launcher разрешает один и тот же корень плагина как из соглашений
Codex, так и из подставленного корня Claude Code.

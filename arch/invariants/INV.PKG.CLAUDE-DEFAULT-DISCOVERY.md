---
id: INV.PKG.CLAUDE-DEFAULT-DISCOVERY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_plugin.py::test_claude_manifest_leaves_skill_discovery_to_the_default_scan
scope: [host, pkg, product]
---

# Манифест Claude не дублирует автоматическое обнаружение

Манифест Claude не объявляет `skills` и `mcpServers`; клиент обнаруживает их в
общем каталоге плагина по стандартному соглашению.

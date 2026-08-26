---
id: INV.PKG.HOST-SHARED-MCP
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_plugin.py::test_packaged_plugin_serves_both_hosts_from_one_directory
scope: [host, pkg, product]
---

# Оба хоста запускают MCP из одного каталога плагина

Один `.mcp.json` разрешает корень через соглашения Codex и Claude Code и
запускает тот же сервер `unica` из общих байтов пакета.

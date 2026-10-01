---
id: INV.WIRE.ONE-SERVER
check:
  - tests/ci/test_package_unica_plugin.py::PackageUnicaPluginTests.test_source_mcp_declares_single_unica_orchestrator
  - tests/ci/test_package_unica_plugin.py::PackageUnicaPluginTests.test_packaged_plugin_serves_both_hosts_from_one_directory
---

# Codex и Claude Code используют один публичный MCP-сервер

В исходном `.mcp.json` объявлен ровно один сервер — `unica`. Встроенные
движки и адаптеры не становятся отдельными публичными MCP-серверами.

Пакет сохраняет общий `.mcp.json` для Codex и Claude Code. Его launcher
разрешает каталог плагина по соглашениям обоих хостов и запускает тот же
сервер из общих файлов. Проверка разрешения корня на POSIX описана
в [правиле общего каталога плагина](../distribution/posix-plugin-root.md).

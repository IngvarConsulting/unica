---
id: INV.PKG.DEV-PACKAGE-ISOLATED
check:
  - tests/ci/test_package_unica_plugin.py::PackageUnicaPluginTests.test_local_debug_mode_remains_current_host_only_and_uses_unica_dev
  - tests/ci/test_package_unica_plugin.py::PackageUnicaPluginTests.test_load_tool_bundles_can_filter_one_release_target
---

# Отладочный пакет содержит только выбранную платформу

Локальный упаковщик включает исполняемые компоненты только для платформы,
явно выбранной через `--local-debug-target`. MCP запускает бинарник этого
пакета напрямую, без установщика bootstrap.

При упаковке для Codex имя каталога по умолчанию — `unica-dev`.
Его можно явно изменить через `--marketplace-name`.

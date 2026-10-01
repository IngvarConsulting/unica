---
id: INV.PKG.COLD-INSTALL-STARTUP-BUDGET
check:
  - tests/ci/test_package_unica_plugin.py::PackageUnicaPluginTests.test_packaged_mcp_declares_its_own_cold_install_startup_budget
---

# Пакет выделяет время на первую загрузку ядра

В сгенерированном `.mcp.json` сервер `unica` объявляет
`startup_timeout_sec` не меньше 600 секунд. Этот срок даёт загрузчику время
получить ядро перед запуском MCP. Он не гарантирует завершения загрузки
при любой скорости сети.

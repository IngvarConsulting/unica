---
id: INV.PKG.COLD-INSTALL-STARTUP-BUDGET
status: active
governs: product
decision: DEC.2026-08-19.CORE-FIRST-ACQUISITION
check: tests/ci/test_package_unica_plugin.py::test_packaged_mcp_declares_its_own_cold_install_startup_budget
scope: [host, pkg, product]
---

# Пакет объявляет бюджет холодной доставки ядра

Сгенерированный `.mcp.json` несёт `startup_timeout_sec` не меньше 600 секунд,
чтобы первая доставка тонкого ядра завершилась до запуска полного MCP. Движки
в этот стартовый путь не входят.

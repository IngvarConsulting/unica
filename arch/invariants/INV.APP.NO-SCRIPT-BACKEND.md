---
id: INV.APP.NO-SCRIPT-BACKEND
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_plugin.py::test_unica_coder_has_no_runtime_operation_script_fallback
scope: [app]
---

# Ядро не откатывается на скрипты операций

Продуктивные Rust-источники не содержат legacy-обработчика и не запускают
Python, PowerShell или shell как запасной backend операций.

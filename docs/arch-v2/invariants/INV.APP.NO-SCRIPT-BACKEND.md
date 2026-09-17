---
id: INV.APP.NO-SCRIPT-BACKEND
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_plugin.py::test_unica_coder_has_no_runtime_operation_script_fallback
scope: [app]
---

# Literal-страж не находит прежний скриптовый backend

В продуктивных Rust-источниках отсутствуют проверяемые литералы legacy-
обработчика и `Command::new` для `python3`, `python`, `bash`, `powershell` и
`pwsh`; каталог `plugins/unica/scripts/legacy` отсутствует.

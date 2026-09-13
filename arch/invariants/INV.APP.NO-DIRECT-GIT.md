---
id: INV.APP.NO-DIRECT-GIT
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_product_contracts.py::test_application_layer_does_not_spawn_git_directly
scope: [app]
---

# Application не содержит прямой literal-запуск git

Префикс продуктивного кода до тестового модуля в каждом Rust-файле application
не содержит литерал `std::process::Command::new("git")`.

---
id: INV.APP.NO-DIRECT-GIT
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_product_contracts.py::test_application_layer_does_not_spawn_git_directly
scope: [app]
---

# Application не запускает git

Продуктивный код слоя application не создаёт процесс `git` напрямую.

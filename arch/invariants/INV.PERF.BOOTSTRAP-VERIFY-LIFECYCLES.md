---
id: INV.PERF.BOOTSTRAP-VERIFY-LIFECYCLES
status: active
governs: product
decision: DEC.2026-08-31.V0-13-SURFACE-FIRST-CUTOVER
check: crates/unica-bootstrap/tests/platform/verification_contract.rs::verify_requires_both_lifecycles_and_the_exact_v13_compatibility_surface
scope: [host, pkg, product]
---

# Проверка runtime проходит оба MCP-жизненных цикла

Bootstrap принимает runtime только после прямого и совместимого жизненных
циклов `initialize` и `tools/list` с ожидаемой публичной поверхностью.

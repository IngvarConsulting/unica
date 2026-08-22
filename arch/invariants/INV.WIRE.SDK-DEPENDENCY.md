---
id: INV.WIRE.SDK-DEPENDENCY
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_product_contracts.py::test_rmcp_dependency_is_owned_by_unica_coder_without_macro_features
scope: [wire]
---

# Зависимость от SDK принадлежит только транспортному package

В `cargo metadata` всего workspace ровно package `unica-coder` прямо зависит от
crates.io-package `rmcp`: без rename и default features, с точным набором
features `server` и `transport-io`.

---
id: INV.WIRE.SDK-TRANSPORT
status: active
governs: process
decision: DEC.2026-08-24.NATIVE-TASK-PROJECTION-SLICE
check: tests/ci/test_product_contracts.py::test_rmcp_transport_is_confined_to_mcp_interface
scope: [wire]
---

# Ссылки на официальный SDK изолированы в interfaces

Среди Git-tracked Rust-файлов под `src` каждого фактического workspace package
из `cargo metadata` продуктивные ссылки на `rmcp` остаются под
`crates/unica-coder/src/interfaces/`. Комментарии, литералы и элементы с точным атрибутом
`#[cfg(test)]` в эту границу не входят.

---
id: INV.WIRE.SDK-TRANSPORT
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_product_contracts.py::test_rmcp_transport_is_confined_to_mcp_interface
scope: [wire]
---

# Ссылки на официальный SDK изолированы в транспортном модуле

Среди Git-tracked Rust-файлов под `src` каждого фактического workspace package
из `cargo metadata` продуктивные ссылки на `rmcp` остаются в
`interfaces/mcp.rs`. Комментарии, литералы и элементы с точным атрибутом
`#[cfg(test)]` в эту границу не входят.

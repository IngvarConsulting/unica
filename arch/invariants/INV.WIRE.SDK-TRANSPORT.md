---
id: INV.WIRE.SDK-TRANSPORT
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_product_contracts.py::test_rmcp_transport_is_confined_to_mcp_interface
scope: [wire]
---

# Ссылки на официальный SDK изолированы в транспортном модуле

Среди отслеживаемых Rust-файлов в `src` корневого package и всех членов Cargo
workspace, включая раскрытые glob-шаблоны и за вычетом `exclude`, продуктивные
ссылки на `rmcp` остаются в `interfaces/mcp.rs`. Комментарии, литералы и элементы
с точным атрибутом `#[cfg(test)]` в эту границу не входят.

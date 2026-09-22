---
id: INV.WIRE.SDK-MODULE-EXPORTS
check:
  - tests/ci/test_product_contracts.py::ProductContractTests.test_rmcp_module_preserves_legacy_public_exports_only
---

# Транспортный модуль сохраняет публичный Rust API

Корневой публичный API `interfaces/mcp.rs` состоит из `MCP_MAX_TOOL_WORKERS`,
`UnicaServer`, `tool_definitions()` и `run_stdio()`. Функция `run_stdio()`
не имеет параметров или обобщений и возвращает `()`.

Модуль не добавляет публичных сущностей и полей, элементов с ограниченной
видимостью (`pub(crate)` и подобных), публичных re-export и type alias,
вложенных public functions или экспортируемых макросов. Элементы с точным
атрибутом `#[cfg(test)]` исключены из проверки этой границы.

---
id: INV.WIRE.SURFACE-RELEASE-ROUTING
check:
  - crates/unica-coder/src/interfaces/mcp.rs::production_mcp_surface_exposes_only_canonical_v13_tools_and_task_compatibility
  - crates/unica-coder/src/interfaces/mcp.rs::a_name_outside_the_canonical_profile_is_refused_without_dispatch
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_catalog_locks_the_eight_domain_contracts_without_publishing_them
---

# Публичный MCP направляет вызовы в канонический демон

Производственный MCP-сервер собирается с одним маршрутом — каноническим
обработчиком демона v0.13. Каталога профиля v0.12 нет. Вызов имени вне
канонического профиля получает отказ `invalid_params` и до обработчика
не доходит.

Состав инструментов задаёт [правило профилей MCP](mcp-tool-profiles.md).

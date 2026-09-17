---
id: CTR.WIRE.CANONICAL-TOOL-PROFILES
check:
  - crates/unica-coder/src/interfaces/mcp.rs::production_mcp_surface_exposes_only_canonical_v13_tools_and_task_compatibility
  - crates/unica-coder/src/interfaces/mcp.rs::surface_profiles_publish_eight_native_or_eleven_compatibility_tools_per_client
---

# Профиль MCP определяет закрытый набор инструментов

Production router публикует `unica.view`, `unica.apply`, `unica.resolve`,
`unica.search`, `unica.check`, `unica.diff`, `unica.run` и `unica.docs`.
Профиль совместимости добавляет только `unica.task.get`, `unica.task.result`
и `unica.task.cancel`; профиль native Tasks их не публикует.

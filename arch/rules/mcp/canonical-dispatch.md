---
id: INV.WIRE.SURFACE-RELEASE-ROUTING
check:
  - crates/unica-coder/src/interfaces/mcp.rs::surface_release_structurally_gates_v12_legacy_dispatch_from_v13_daemon_dispatch
---

# Публичный MCP направляет вызовы в канонический демон

Производственный профиль v0.13 передаёт вызовы каноническому обработчику
демона и не возвращается к исполнению через legacy v0.12. Legacy-маршрут
используется только в изолированных тестах, которые различают эти два пути.

Состав инструментов задаёт [правило профилей MCP](mcp-tool-profiles.md).

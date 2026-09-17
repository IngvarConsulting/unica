---
id: CTR.WIRE.CANONICAL-TOOL-PROFILES
check:
  - crates/unica-coder/src/interfaces/mcp.rs::production_mcp_surface_exposes_only_canonical_v13_tools_and_task_compatibility
  - crates/unica-coder/src/interfaces/mcp.rs::surface_profiles_publish_eight_native_or_eleven_compatibility_tools_per_client
---

# Режим MCP определяет, какие инструменты видит клиент

Unica предоставляет восемь основных инструментов: `unica.view`, `unica.apply`,
`unica.resolve`, `unica.search`, `unica.check`, `unica.diff`, `unica.run`
и `unica.docs`.

В режиме со встроенным механизмом заданий MCP (`native Tasks`) клиент видит
только эти восемь инструментов. В режиме совместимости добавляются три
инструмента работы с заданиями: `unica.task.get`, `unica.task.result`
и `unica.task.cancel`. Итого клиент получает ровно восемь или одиннадцать
инструментов, в зависимости от режима.

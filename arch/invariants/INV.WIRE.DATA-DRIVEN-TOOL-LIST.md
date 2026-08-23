---
id: INV.WIRE.DATA-DRIVEN-TOOL-LIST
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/interfaces/mcp.rs::application_registry_owns_tool_names_descriptions_and_wire_schemas
scope: [wire]
---

# Application владеет контрактами проекции инструментов

`ToolSpec` содержит уникальные имена и непустые описания, а проекция MCP
сохраняет число, порядок и имена записей, не выводит эти описания на schema-only
провод и использует построенные application входные и выходные схемы после
удаления вложенных описаний схемы.

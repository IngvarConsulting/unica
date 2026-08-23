---
id: INV.SURFACE.DIAGNOSTIC-TARGET
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/interfaces/mcp.rs::tool_definitions_expose_logical_diagnostics_action_union
scope: [wire]
---

# Схема диагностики публикует логическую цель

Каждая ветвь схемы `unica.code.diagnostics` содержит `action`, `sourceSet` и
`cwd` и не возвращает снятые селекторы `sourceDir`, `mode`, `path` и `codes`.

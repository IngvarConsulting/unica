---
id: INV.WIRE.DIRECT-FIRST-LIFECYCLE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/interfaces/mcp.rs::modern_direct_first_tools_list_pages_through_the_full_registry
scope: [wire]
---

# Direct-first tools/list отдаёт непересекающиеся ограниченные страницы

Прямой первый `tools/list` ревизии `2026-07-28` отдаёт страницы не длиннее 25
записей без описаний; обход курсоров возвращает столько уникальных имён,
сколько записей содержит application-реестр.

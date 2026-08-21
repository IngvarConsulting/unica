---
id: INV.WIRE.DATA-DRIVEN-TOOL-LIST
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/interfaces/mcp.rs::tools_list_round_trips_the_data_driven_registry
scope: [wire]
---

# tools/list отображает реестр application без расхождений

Список инструментов, прошедший через транспорт, совпадает по именам и числу
записей с реестром application и не содержит дублей.

---
id: INV.SURFACE.SOURCE-TOOL-SPECS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::source_resource_tools_are_read_only_and_have_no_cache_or_event_effects
scope: [wire]
---

# ToolSpec ресурсных операций объявляет read-only и пустой cache access

Записи `unica.source.resources` и `unica.source.read` в application-реестре
имеют немутирующее исполнение, пустые множества чтения и записи кеша и
соответствующие `SourceResources` handlers; `unica.source.apply` не объявлен.

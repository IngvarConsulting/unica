---
id: INV.SURFACE.NO-RAW-ADAPTER-ARGS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/interfaces/mcp.rs::no_public_tool_schema_exposes_raw_adapter_args
scope: [wire]
---

# Публичные схемы не показывают сырой args адаптера

Ни одна объектная ветвь входной схемы инструмента, построенной из application-
реестра, не содержит свойство `args`.

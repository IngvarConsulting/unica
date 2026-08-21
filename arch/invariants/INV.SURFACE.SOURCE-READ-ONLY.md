---
id: INV.SURFACE.SOURCE-READ-ONLY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/mod.rs::source_resource_tools_are_read_only_and_have_no_cache_or_event_effects
scope: [wire]
---

# Ресурсные инструменты источника только читают

`unica.source.resources` и `unica.source.read` не мутируют источник и кеш, а
публичный `unica.source.apply` отсутствует.

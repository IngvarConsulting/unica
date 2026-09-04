---
id: INV.SURFACE.SOURCE-TOOL-SPECS
status: active
governs: product
decision: DEC.2026-09-04.SKILLS-CANONICAL-SURFACE
check: crates/unica-coder/src/application/mod.rs::source_resource_tools_are_read_only_and_have_no_cache_or_event_effects
scope: [wire]
---

# ToolSpec читателя объявляет пустую запись кеша и не публикует событие

Каждая немутирующая запись application-реестра имеет пустое множество записи
кеша и не публикует доменное событие; `unica.source.apply` в реестре не
объявлен, потому что правку BSL выполняет `unica.code.patch`.

---
id: INV.SURFACE.NAMESPACE
status: active
decision: DEC.2026-08-18.EIGHT-ENTRIES
check: crates/unica-coder/src/application/tool_contracts.rs::every_public_tool_lives_in_the_unica_namespace
scope: [wire]
---

# Публичные инструменты живут в пространстве unica
Каждый публичный инструмент живёт в пространстве `unica.`. Хост, сводящий
несколько серверов в один список, иначе не отличит наш инструмент от чужого.

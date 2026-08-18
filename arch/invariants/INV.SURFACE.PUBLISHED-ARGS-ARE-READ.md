---
id: INV.SURFACE.PUBLISHED-ARGS-ARE-READ
status: active
decision: DEC.2026-08-18.EIGHT-ENTRIES
check: crates/unica-coder/src/application/tool_contracts.rs::narrowed_tools_publish_a_working_surface_not_the_legacy_union
scope: [wire]
---

# Публикуется то, что обработчик читает
Инструмент публикует аргументы, которые его обработчик читает, и адреса,
объявленные его дескриптором. Аргумент, который никто не читает, рекламирует
рычаг, ничего не выбирающий, и оплачивается каждым сеансом.

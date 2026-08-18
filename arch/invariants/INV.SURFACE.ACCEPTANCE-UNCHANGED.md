---
id: INV.SURFACE.ACCEPTANCE-UNCHANGED
status: active
decision: DEC.2026-08-18.EIGHT-ENTRIES
check: crates/unica-coder/src/application/tool_contracts.rs::narrowing_publication_keeps_the_legacy_union_accepted
scope: [wire]
---

# Сужение публикации не сужает приём
Сужение публикуемой схемы не сужает приём: клиент, посылающий исторический
аргумент, продолжает работать. Публикация — про то, чему учат модель; приём —
про то, что не ломается.

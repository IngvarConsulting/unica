---
id: INV.SURFACE.ACCEPTANCE-UNCHANGED
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/tool_contracts.rs::every_narrowed_reader_publishes_its_exact_argument_set
scope: [wire]
---

# Сужение публикации не сужает приём
Сужение публикуемой схемы не сужает приём: клиент, посылающий исторический
аргумент, продолжает работать. Публикация — про то, чему учат модель; приём —
про то, что не ломается.

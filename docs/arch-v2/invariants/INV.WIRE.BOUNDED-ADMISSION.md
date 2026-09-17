---
id: INV.WIRE.BOUNDED-ADMISSION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/interfaces/mcp.rs::admission_is_bounded_and_reusable
scope: [wire]
---

# Приём вызовов ограничен

Число одновременно исполняемых вызовов ограничено, и освободившееся место
переиспользуется.

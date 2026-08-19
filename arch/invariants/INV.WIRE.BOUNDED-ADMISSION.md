---
id: INV.WIRE.BOUNDED-ADMISSION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/interfaces/mcp.rs
scope: [wire]
---

# Приём вызовов ограничен, отмена кооперативна

Число одновременно исполняемых вызовов ограничено, а отмена доходит до исполнителя и
завершает работу, а не бросает её висеть.

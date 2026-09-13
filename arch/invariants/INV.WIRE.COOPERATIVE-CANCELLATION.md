---
id: INV.WIRE.COOPERATIVE-CANCELLATION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/interfaces/mcp.rs::ping_stays_responsive_and_cancellation_reaches_the_tool
scope: [wire]
---

# Отмена доходит до исполнителя

Отмена достигает работающего инструмента и завершает работу, а не бросает её
висеть; сервер при этом остаётся отзывчивым.

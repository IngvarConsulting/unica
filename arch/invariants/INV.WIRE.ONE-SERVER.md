---
id: INV.WIRE.ONE-SERVER
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/interfaces/mcp.rs
scope: [wire]
---

# Модель видит один сервер и ни одного движка

Публичная граница — единственный MCP-сервер с именем `unica`. Встроенные движки и
серверы-адаптеры в видимую модели маршрутизацию не попадают.

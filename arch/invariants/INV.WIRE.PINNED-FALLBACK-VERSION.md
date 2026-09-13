---
id: INV.WIRE.PINNED-FALLBACK-VERSION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/interfaces/mcp.rs::legacy_unknown_offer_falls_back_to_pinned_version
scope: [wire]
---

# Legacy fallback закреплён на 2025-11-25

Неизвестное предложение версии в legacy-инициализации получает ответ
`2025-11-25`, а не подвижную последнюю версию зависимости.

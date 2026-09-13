---
id: CTR.WIRE.LIST-CACHE-FIELDS
status: active
governs: product
version: 1
decision: DEC.2026-08-21.LIST-CACHE-FIELDS
producer: crates/unica-coder/src/interfaces/mcp.rs
consumers: [host]
check: crates/unica-coder/src/interfaces/mcp.rs::modern_list_results_carry_required_cache_fields_and_legacy_stays_clean
scope: [wire]
---

# Современный list несёт cache-поля, legacy сохраняет прежнюю форму

Современная ветка `tools/list` несёт обязательные поля SEP-2549 `ttlMs` и
`cacheScope`; legacy-ветка не получает эти поля и сохраняет прежнюю форму.

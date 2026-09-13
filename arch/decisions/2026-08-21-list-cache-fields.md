---
id: DEC.2026-08-21.LIST-CACHE-FIELDS
status: active
governs: product
realized: crates/unica-coder/src/interfaces/mcp.rs::modern_list_results_carry_required_cache_fields_and_legacy_stays_clean
establishes: [CTR.WIRE.LIST-CACHE-FIELDS]
---

# Cache-fields split modern and legacy tools/list responses

**Решение.** Современный `tools/list` несёт `ttlMs` и `cacheScope`; legacy-ответ
эти поля не получает.

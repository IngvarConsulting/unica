---
id: CTR.WIRE.LIST-CACHE-FIELDS
check:
  - crates/unica-coder/src/interfaces/mcp.rs::modern_list_results_carry_required_cache_fields_and_legacy_stays_clean
---

# Cache-поля tools/list зависят от протокола

В ответе `tools/list` для MCP `2026-07-28` есть `ttlMs: 0` и
`cacheScope: "private"`. Для legacy-протокола `2025-11-25` эти поля отсутствуют.

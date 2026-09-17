---
id: CTR.WIRE.LIST-CACHE-FIELDS
check:
  - crates/unica-coder/src/interfaces/mcp.rs::modern_list_results_carry_required_cache_fields_and_legacy_stays_clean
---

# Настройки кеширования списка инструментов зависят от версии MCP

Когда клиент запрашивает список инструментов (`tools/list`), Unica отвечает
в формате используемой версии MCP.

Для версии `2026-07-28` ответ содержит настройки кеширования `ttlMs: 0` и
`cacheScope: "private"`. Для версии `2025-11-25` этих двух полей в ответе
быть не должно, в том числе со значением `null`.

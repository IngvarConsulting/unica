---
id: INV.CACHE.OVERRIDE-PRIORITY
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-bootstrap/src/host/runtime_cache.rs::the_explicit_override_outranks_every_host_source
scope: [cache]
---

# Явный runtime cache override опережает источники хоста

Развёрнутое значение `UNICA_RUNTIME_CACHE_DIR` выбирается как есть и опережает
`CLAUDE_PLUGIN_DATA`, `CODEX_HOME` и пользовательский home-каталог.

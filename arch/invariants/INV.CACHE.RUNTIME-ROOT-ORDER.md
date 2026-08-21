---
id: INV.CACHE.RUNTIME-ROOT-ORDER
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-bootstrap/src/host/runtime_cache.rs::the_explicit_override_outranks_every_host_source
scope: [cache]
---

# Явный корень runtime-кеша имеет высший приоритет

Развёрнутое значение `UNICA_RUNTIME_CACHE_DIR` выбирается без добавления
суффикса и опережает все каталоги данных и home-каталоги хостов.

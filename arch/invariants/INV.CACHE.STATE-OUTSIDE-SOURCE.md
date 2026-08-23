---
id: INV.CACHE.STATE-OUTSIDE-SOURCE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/workspace_services.rs::bsl_analyzer_cache_stays_outside_a_workspace_wide_source_root
scope: [cache]
---

# Кеш анализатора не попадает в индексируемый источник

Если настроенный корень кеша находится внутри корня исходников, кеш анализатора
переносится наружу и сохраняет ключ нормализованной идентичности источника.

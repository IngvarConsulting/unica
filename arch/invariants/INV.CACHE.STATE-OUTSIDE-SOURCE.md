---
id: INV.CACHE.STATE-OUTSIDE-SOURCE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/workspace_services.rs::bsl_analyzer_cache_stays_outside_a_workspace_wide_source_root
scope: [cache]
---

# Состояние поставщика лежит вне индексируемого источника

Постоянное состояние поставщика не индексирует само себя, изолируется связанным рабочим
деревом, а несовместимая версия получает новое поколение вместо миграции старого.

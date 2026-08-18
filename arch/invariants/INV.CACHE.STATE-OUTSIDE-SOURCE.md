---
id: INV.CACHE.STATE-OUTSIDE-SOURCE
status: active
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/workspace_services.rs
scope: [cache]
---

# Состояние поставщика лежит вне индексируемого источника

Постоянное состояние поставщика не индексирует само себя, изолируется связанным рабочим
деревом, а несовместимая версия получает новое поколение вместо миграции старого.

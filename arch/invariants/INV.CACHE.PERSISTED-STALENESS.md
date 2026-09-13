---
id: INV.CACHE.PERSISTED-STALENESS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/workspace_state.rs::bsl_index_read_reflects_real_index_status_instead_of_lazy_rebuild
scope: [cache]
---

# Инвалидация индекса остаётся видимой при следующем чтении

После события изменения модуля следующий отчёт чтения по-прежнему называет
`bsl_index` устаревшим и не объявляет его молча пересобранным.

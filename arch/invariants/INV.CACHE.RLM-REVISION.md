---
id: INV.CACHE.RLM-REVISION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/workspace_services.rs::rlm_execute_discards_output_when_source_changes_before_fake_execute_returns
scope: [cache]
---

# RLM не публикует результат устаревшей ревизии

Если источник изменился во время исполнения, ответ помечается устаревшим,
данные поставщика отбрасываются, а обслуживание планируется для новой ревизии.

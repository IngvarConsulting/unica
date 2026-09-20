---
id: INV.CACHE.RLM-STATE-LOCATION
check:
  - crates/unica-coder/src/infrastructure/workspace_index.rs::rlm_provider_state_moves_outside_a_workspace_wide_source_root
  - crates/unica-coder/src/infrastructure/workspace_index.rs::safe_root_failure_never_falls_back_to_generation_markers_inside_sources
---

# Состояние RLM размещается вне индексируемых исходников

Рассчитанный Unica постоянный каталог RLM находится вне корня исходников,
даже когда исходники занимают весь проект. Если безопасный каталог определить
нельзя, отказ не заменяется записью файлов индекса, состояния или блокировки
внутрь исходников. Разделение данных задаёт
[правило изоляции источников и профилей](../workspace/source-profile-state-isolation.md).

Проверки охватывают расчёт путей в Unica;
они не проверяют произвольные записи внешнего процесса RLM.

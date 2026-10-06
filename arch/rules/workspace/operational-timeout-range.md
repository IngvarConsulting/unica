---
id: INV.CONFIG.OPERATIONAL-TIMEOUT-RANGE
check:
  - crates/unica-coder/src/infrastructure/operational_config.rs::positive_timeout_values_are_accepted_without_clamping
  - crates/unica-coder/src/infrastructure/operational_config.rs::all_timeout_fields_reject_invalid_types_and_ranges
  - crates/unica-coder/src/infrastructure/operational_config.rs::final_snapshot_enforces_provider_deadlines_not_exceeding_total
  - crates/unica-coder/src/infrastructure/operational_config.rs::cross_layer_constraint_is_attributed_to_the_later_override
  - crates/unica-coder/src/infrastructure/operational_config.rs::missing_files_use_exact_compiled_defaults
  - crates/unica-coder/src/infrastructure/internal_adapters.rs::diagnostics_analyze_uses_workspace_operational_config_default
---

# Файловые сроки не ограничиваются значениями по умолчанию

Операционные сроки в файлах настроек задаются целыми секундами от 1
в пределах числового типа. Значение по умолчанию не становится потолком:
большее допустимое значение сохраняется без обрезания.

Срока анализа диагностики по умолчанию нет: без значения в файлах анализ
идёт до конца, а заданное значение соблюдается. Сроки поиска пока имеют
значения по умолчанию; их снятие —
[#1254](https://github.com/IngvarConsulting/unica/issues/1254).

После слияния слоёв сроки RLM и git-grep не должны превышать общий срок
поиска. Противоречивое сочетание отклоняется. Это правило относится
к файловым настройкам; явный аргумент диагностики имеет свой диапазон.

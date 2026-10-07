---
id: INV.ANALYSIS.ANALYZER-JSONL-COMPLETION
check:
  - crates/unica-coder/src/infrastructure/diagnostics_jsonl.rs::complete_stream_projects_typed_data_without_upstream_shape
  - crates/unica-coder/src/infrastructure/diagnostics_jsonl.rs::only_start_is_pending_and_file_without_done_is_incomplete
  - crates/unica-coder/src/infrastructure/diagnostics_jsonl.rs::invalid_grammar_and_totals_fail_closed_without_partial_items
  - crates/unica-coder/src/infrastructure/diagnostics_jsonl.rs::duplicate_start_done_and_normalized_path_are_invalid
  - crates/unica-coder/src/infrastructure/diagnostics_jsonl.rs::diagnostic_validation_rejects_unknown_severity_tags_and_range
  - crates/unica-coder/src/infrastructure/diagnostics_jsonl.rs::done_baseline_is_a_named_filter_and_only_a_broken_one_is_incomplete
  - crates/unica-coder/src/infrastructure/diagnostics_baseline.rs::baseline_verdict_is_a_filter_unless_broken_or_unclassified
---

# Оборванный или противоречивый ответ анализатора не означает чистый код

Разборщик JSONL принимает начало анализа, уникальные файлы и завершение
с согласованными итогами. Неизвестные события и поля, повторы, неверные
уровни и диапазоны диагностик приводят к отказу.

Одно начало без результатов отличается от потока файлов без завершения.
Ни тот, ни другой ответ не выдаёт частичные находки за завершённый анализ.

Сводка базовой линии диагностик в завершении разбирается по полям апстрима,
состояние — закрытым набором. Поток, в котором базовая линия скрыла все
находки, не объявляется пустым: подавление называется фактом.

Правило относится к протоколу поставщика, а не к форме публичного ответа MCP.

---
id: INV.ANALYSIS.ANALYZER-PROCESS-FAILURE
check:
  - crates/unica-coder/src/infrastructure/internal_adapters.rs::diagnostics_analyze_process_failure_takes_priority_over_invalid_stream
  - crates/unica-coder/src/infrastructure/internal_adapters/tests/medium_diagnostics_process_failure.rs::invalid_jsonl_does_not_stop_draining_owned_process_pipes
---

# Отмена и сбой процесса анализа не превращаются в ошибку JSONL

Отмена, превышение срока и неуспешное завершение анализатора сохраняют
свои причины, даже если его JSONL оборван или повреждён. Находки
такого процесса не выдаются как завершённый результат анализа.

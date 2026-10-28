---
id: INV.SOURCE.BSL-SYNTAX-ERROR-READING
check:
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::broken_bsl_body_keeps_text_and_reports_full_module_syntax_in_ranges
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::broken_bsl_structure_refuses_with_reason_and_body_route_without_empty_structure
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::broken_bsl_many_errors_keep_body_available_with_bounded_diagnostics
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::broken_bsl_body_uses_buffer_syntax_and_preserves_cancellation_and_disk
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::broken_bsl_projection_cache_retains_invalid_evidence_only_for_its_revision
  - crates/unica-coder/src/infrastructure/daemon/v13_read_modes.rs::broken_bsl_typed_evidence_gates_dictionary_and_direct_contract_together
  - crates/unica-bindings-integration-tests/tests/projection.rs::broken_bsl_read_outcome_keeps_strict_projection_closed
  - crates/unica-coder/tests/platform/code_patch_composition.rs::a_module_that_did_not_parse_names_its_errors_as_the_source
gap: https://github.com/IngvarConsulting/unica/issues/944
---

# Синтаксическая ошибка не скрывает текст BSL-модуля

При синтаксической ошибке `Body` доступен как исходный текст с диагностикой.
Структурные методы и события недоступны до исправления: отсутствие надёжной
структуры не выдаётся за пустой корректный модуль.

Чтение сохраняет ограничения доступа, пути, размера и срока операции и не меняет
исходники. Зависимые изменения остановлены; восстановление выполняется по
[правилу согласованного отката](damaged-object-recovery.md).

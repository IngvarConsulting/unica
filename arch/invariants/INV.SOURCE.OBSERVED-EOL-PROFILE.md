---
id: INV.SOURCE.OBSERVED-EOL-PROFILE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check:
  - crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs::snapshot_classifies_no_line_endings
  - crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs::snapshot_classifies_uniform_lf_and_terminal_newline
  - crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs::snapshot_classifies_uniform_crlf_and_terminal_newline
  - crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs::snapshot_classifies_uniform_cr_and_terminal_newline
  - crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs::snapshot_classifies_mixed_endings_with_exact_counts
  - crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs::snapshot_reports_missing_terminal_newline
  - crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs::preserve_prefers_local_context_for_mixed_source
  - crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs::observed_resolution_serves_no_eol_source_with_explicit_lf
  - crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs::observed_resolution_preserves_uniform_profile_and_prefers_local
  - crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs::observed_resolution_rejects_mixed_profile_without_local_context
scope: [product, source]
---

# Профиль перевода строки выводится из наблюдаемых байтов

Снимок различает отсутствие EOL, единый LF, CRLF или CR и смешанный профиль с
точными счётчиками и признаком завершающего EOL. `Preserve` использует локальный
контекст или единый профиль и отказывает на недоказуемом смешанном выборе;
явная политика LF обслуживает текст без EOL.

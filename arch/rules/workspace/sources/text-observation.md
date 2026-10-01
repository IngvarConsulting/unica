---
id: INV.SOURCE.OBSERVED-BYTES
check:
  - crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs::snapshot_preserves_raw_bytes_and_excludes_one_bom_from_text
  - crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs::snapshot_treats_only_the_first_bom_as_preamble
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
---

# Текстовый снимок сохраняет байты и наблюдаемые переводы строк

Снимок хранит исходные байты без изменений. Из текста для редактора
отделяется только первый UTF-8 BOM; второй, если он есть, остаётся в тексте.

Переводы строк определяются по содержимому: их нет, используется один вид
LF, CRLF или CR либо виды смешаны. Для смешанного текста сохраняется точное
число переводов каждого вида. Отдельно отмечается перевод в конце текста.

При сохранении формата перевода строк выбирается вид в месте правки,
а без такого контекста — единый вид во всём тексте. Смешанный текст без
локального контекста отклоняется как неоднозначный. Для текста без переводов
строк явно выбирается LF.

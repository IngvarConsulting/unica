---
id: INV.SOURCE.CODE-PATCH-EOL
check:
  - crates/unica-bsl/src/text.rs::code_patch_rejects_lone_cr_instead_of_inventing_or_gaining_an_eol_policy
  - crates/unica-coder/tests/platform/code_patch_composition.rs::code_patch_without_any_source_eol_uses_lf_for_preview_apply_and_repeat_noop
  - crates/unica-coder/tests/platform/code_patch_composition.rs::mixed_eol_apply_preserves_untouched_bytes_and_uses_target_eol
  - crates/unica-bsl/src/text.rs::unified_diff_round_trips_crlf_and_missing_terminal_eol
---

# Правка BSL сохраняет переводы строк вне изменяемого участка

Правка BSL оставляет нетронутые байты исходника без изменений, даже если
в файле смешаны LF и CRLF. Добавляемый текст использует переводы строк
целевого метода. Для исходника без переводов строк выбирается LF.
Одиночный CR приводит к отказу, а не к нормализации всего файла.

Сформированный diff при применении точно воспроизводит подготовленный
результат, в том числе для CRLF и исходного файла без завершающего перевода
строки.

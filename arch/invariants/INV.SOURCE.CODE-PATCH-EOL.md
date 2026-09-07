---
id: INV.SOURCE.CODE-PATCH-EOL
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check:
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::code_patch_rejects_lone_cr_instead_of_inventing_or_gaining_an_eol_policy
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::code_patch_without_any_source_eol_uses_lf_for_preview_apply_and_repeat_noop
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::mixed_eol_apply_preserves_untouched_bytes_and_uses_target_eol
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::unified_diff_round_trips_crlf_and_missing_terminal_eol
scope: [product, source]
---

# Code patch сохраняет наблюдаемую локальную форму EOL

`unica.code.patch` сохраняет нетронутые байты смешанного файла и использует EOL
целевого метода, обслуживает источник без EOL явным LF, сохраняет отсутствие
завершающего перевода и отказывает на одиночном CR вместо глобальной
нормализации.

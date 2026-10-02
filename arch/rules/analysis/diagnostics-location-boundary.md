---
id: INV.SAFETY.DIAGNOSTIC-LOCATION-BOUNDARY
check:
  - crates/unica-coder/src/infrastructure/internal_adapters.rs::diagnostics_analyze_preserves_cyrillic_paths_through_typed_jsonl
  - crates/unica-coder/src/infrastructure/diagnostics.rs::diagnostic_location_distinguishes_unaddressable_owner_and_unproven_owner
  - crates/unica-coder/src/infrastructure/diagnostics.rs::diagnostic_location_rejects_escape_without_leaking_the_raw_handle
  - crates/unica-coder/src/infrastructure/diagnostics.rs::diagnostics_windows_normalizes_separators_unicode_file_uri_and_dot_segments
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_out_of_scope_handle_still_costs_the_whole_provider_section
  - crates/unica-coder/src/application/diagnostics.rs::diagnostics_public_result_redacts_provider_controlled_physical_paths
---

# Диагностика не выдаёт внешний файл за часть выбранных исходников

Ресурс диагностики связывается с логическим адресом. Если адрес доказать нельзя,
допустим только относительный путь внутри разрешённой области с явной причиной
неадресуемости и доказанным владельцем, когда он известен. Разделители пути,
кириллица, пробелы и файловый URI не меняют логическую цель.

Выход за область исходников делает недостоверной всю секцию поставщика:
его находки отбрасываются, секция получает `Failed`. Исходный внешний путь
не раскрывается. Физические пути скрываются также в сообщениях диагностик
и ошибок поставщика; ссылка на описание правила при этом сохраняется.

---
id: INV.SOURCE.WRITE-TARGET-KIND
check:
  - crates/unica-coder/src/infrastructure/daemon/code_target_tests.rs::canonical_code_edits_write_only_at_a_module_address
  - crates/unica-coder/src/infrastructure/daemon/code_target_tests.rs::canonical_code_execution_refuses_a_module_owner_changed_after_preview
  - crates/unica-coder/tests/platform/code_patch_composition.rs::staged_code_preserves_module_owner_support_format_and_preimage_guards
  - crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs::platform_xml_target_kind_policy_table_is_closed
  - crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs::platform_xml_source_root_handle_revalidates_without_widening
  - crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs::platform_xml_source_target_revalidation_rejects_changed_descriptor_identity
  - crates/unica-coder/tests/platform/code_patch_composition.rs::code_patch_refuses_a_metadata_object_address
---

# Цель правки BSL должна указывать на модуль

Канонические `code.insert` и `code.replace` пишут только в модуль: по его
адресу `Kind.Name.Module.<Role>` или `Module.<Role>`, по его `Body` или методу
`Method.M`, а замена ещё и по телу метода `Method.M.Body`. Общий модуль сам
является модулем, поэтому `CommonModule.Name` — адрес модуля, а не его
владельца.

Адрес корня набора, объекта метаданных, ветки `Module` и любого узла, который
не называет один модуль, отклоняется до плана с кодом `bad_value` и без записи.
Если у узла есть модули, `next` ведёт к его ветке `Module` с их адресами.
Прежний `code.patch` отклоняет такие адреса с кодом `TargetKindMismatch`.

Описание владельца модуля, изменённое после плана, отклоняет его исполнение
с `stale_revision`; модуль остаётся прежним.
Резолвер целей сохраняет ограничение по виду: повторная проверка выполняется
под тем же ограничением и отклоняет цель, если описание объекта уже указывает
на другую сущность. Расширение возможностей поиска само по себе не расширяет
право записи.

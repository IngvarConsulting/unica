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

Правки кода (`code.insert`, `code.replace`) пишут только в один модуль, цель
задаётся адресом модуля или его метода. Адрес, который не называет один
модуль, отклоняется до плана без записи; если у узла есть модули, отказ
ведёт к их адресам. Если описание владельца модуля изменилось после плана,
исполнение отказывает и модуль не меняется.

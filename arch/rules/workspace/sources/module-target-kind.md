---
id: INV.SOURCE.WRITE-TARGET-KIND
check:
  - crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs::platform_xml_target_kind_policy_table_is_closed
  - crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs::platform_xml_source_root_handle_revalidates_without_widening
  - crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs::platform_xml_source_target_revalidation_rejects_changed_descriptor_identity
  - crates/unica-coder/src/infrastructure/native_operations/code.rs::code_patch_refuses_a_metadata_object_address
---

# Цель правки BSL должна указывать на модуль

При разрешении цели BSL-патча принимается адрес модуля. Адрес объекта
метаданных или корня исходников отклоняется с кодом `TargetKindMismatch`.

Разрешённая цель сохраняет ограничение по виду. Повторная проверка выполняется
под тем же ограничением и отклоняет цель, если описание объекта уже указывает
на другую сущность. Расширение возможностей поиска само по себе не расширяет
право записи.

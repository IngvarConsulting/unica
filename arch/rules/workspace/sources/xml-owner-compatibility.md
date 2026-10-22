---
id: INV.SOURCE.EXACT-VERSION
check:
  - crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs::readable_child_graph_does_not_authorize_materializing_unsupported_descriptors
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/dcs_mxl_guard_tests.rs::canonical_dcs_supported_source_still_requires_each_exact_owner_version
  - crates/unica-coder/src/infrastructure/format_guard.rs::code_patch_inside_older_source_set_uses_the_same_format_boundary
  - crates/unica-coder/src/infrastructure/platform_xml_owner.rs::existing_form_content_resolves_exact_wrapper_and_source_set_owners
---

# Изменение требует совместимости цели и её владельцев формата

При изменении цели проверяются её собственная версия формата, если она
сама задаёт формат, и версии её владельцев — XML-документов, определяющих
формат именно её объекта и набора исходников.

Неподдерживаемая версия любого из этих документов блокирует изменение.
Совместимость другого владельца не заменяет эту проверку. Отсутствие
собственной версии у цели не отменяет проверку владельцев; допуск к чтению
не разрешает запись несовместимого дескриптора.

---
id: INV.SOURCE.METADATA-MUTATION-VALIDITY
check:
  - crates/unica-metadata/src/operations.rs::tabular_standard_attribute_names_are_rejected_by_all_name_constructors
  - crates/unica-coder/src/infrastructure/daemon/metadata_can_tests.rs::tabular_standard_names_refuse_add_copy_and_rename_without_source_writes
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_preview_refuses_creating_objects_without_the_required_final_composition
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_preview_refuses_removing_the_last_required_register_field
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_preview_refuses_clearing_a_web_service_namespace
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_create_then_complete_registers_previews_and_publishes_minimal_composition
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_create_then_set_namespace_previews_and_publishes_a_web_service
gap: https://github.com/IngvarConsulting/unica/issues/931
---

# Успешное изменение средствами Unica оставляет корректный объект

После успешного создания или изменения методом Unica затронутый объект
должен быть корректен в границах поддерживаемого формата и контракта
операции. Некорректный результат не публикуется как успешное изменение.
Это не обещание проверки всех прикладных правил или всего проекта.

Повреждение, внесённое методом Unica, является дефектом Unica.
Правило не гарантирует корректность последующих внешних правок.

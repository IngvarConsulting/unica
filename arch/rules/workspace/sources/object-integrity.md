---
id: INV.SOURCE.OBJECT-INTEGRITY-AT-PUBLICATION
check:
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_preview_refuses_creating_objects_without_the_required_final_composition
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_preview_refuses_removing_the_last_required_register_field
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_preview_refuses_clearing_a_web_service_namespace
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_create_then_complete_registers_previews_and_publishes_minimal_composition
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_create_then_set_namespace_previews_and_publishes_a_web_service
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_integrity_uses_the_final_batch_after_an_intervening_other_family
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_creation_of_a_calculation_register_without_a_resource_remains_allowed
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_object_removal_does_not_validate_a_deleted_descriptor
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_integrity_checks_own_extension_objects_but_preserves_inherited_composition
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_integrity_rejects_a_foreign_borrowing_marker_without_rejecting_a_metadata_prefix
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/object_integrity_tests.rs::canonical_integrity_does_not_scan_unrelated_incomplete_objects
gap: https://github.com/IngvarConsulting/unica/issues/931
---

# Обязательный состав объекта проверяется перед публикацией

В итоговом образе регистра сведений или накопления должен быть хотя бы
один реквизит, ресурс или измерение. Регистр бухгалтерии требует хотя бы
одного ресурса, веб-сервис — непустого пространства имён.

Условия проверяются после всех операций пакета. Замена единственного
измерения через удаление и добавление в одном пакете допустима.
Нарушение выявляется при preview и блокирует публикацию.

Это локальная проверка объекта. Она не требует готовности всей
конфигурации и не разрешает придумывать недостающие данные.

Для заимствованного объекта учитывается состав родителя. Подключение
проверки итогового унаследованного состава остаётся разрывом из `gap`.

---
id: INV.SOURCE.NO-FORMAT-MIGRATION
status: active
governs: product
decision: DEC.2026-08-21.SINGLE-WRITABLE-PLATFORM-XML-PROFILE
check:
  - crates/unica-coder/src/application/tool_contracts.rs::native_mutation_surface_has_exact_operations_and_schemas
  - crates/unica-coder/src/infrastructure/format_guard.rs::public_platform_xml_mutators_have_closed_pre_side_effect_format_refusal
  - crates/unica-coder/src/infrastructure/format_guard.rs::dcs_edit_blocks_old_external_source_set_via_owner_descriptor
  - crates/unica-coder/src/infrastructure/format_guard.rs::cf_init_public_guard_blocks_newer_existing_post_validation_dependency
scope: [source]
---

# Нативная поверхность не мигрирует формат

Точные имена операций и канонические рекурсивные отпечатки полных схем всех 25
публичных нативных и типизированных XML-мутаторов замкнуты. Каждая нативная
операция имеет дескриптор общего гейта до обработчика, а все три типизированных
metadata-мутатора отдельно проходят публичную проверку отказа на старом и новом
профиле. Отказ сохраняет исходные байты; отдельного пути миграции нет.

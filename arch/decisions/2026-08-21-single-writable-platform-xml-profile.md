---
id: DEC.2026-08-21.SINGLE-WRITABLE-PLATFORM-XML-PROFILE
status: active
governs: product
realized:
  - crates/unica-coder/src/infrastructure/format_guard.rs::single_writable_platform_xml_profile_is_exact
  - crates/unica-coder/src/infrastructure/format_guard.rs::missing_root_version_is_classified_as_1_0
  - crates/unica-coder/src/infrastructure/format_guard.rs::mxl_info_warns_old_external_source_set_via_owner_descriptor
  - crates/unica-coder/src/infrastructure/format_guard.rs::newer_dump_warns_for_read_only_with_roadmap_copy
  - crates/unica-coder/src/infrastructure/format_guard.rs::unknown_version_bearing_roots_are_rejected_by_the_closed_policy_catalog
  - crates/unica-coder/src/infrastructure/format_guard.rs::valid_standalone_mxl_without_owner_version_is_not_an_old_dump
  - crates/unica-coder/src/infrastructure/format_guard.rs::version_owning_target_cannot_hide_behind_supported_source_set_owner
  - crates/unica-coder/src/infrastructure/format_guard.rs::versionless_known_standalone_form_is_classified_as_1_0_owner
  - crates/unica-coder/src/infrastructure/format_guard.rs::xdto_guard_empty_handler_resolution_is_a_contract_error
  - crates/unica-coder/src/infrastructure/platform_xml_owner.rs::equal_depth_source_set_owners_are_ambiguous_for_existing_and_new_outputs
  - crates/unica-coder/src/application/tool_contracts.rs::native_mutation_surface_has_exact_operations_and_schemas
  - crates/unica-coder/src/infrastructure/format_guard.rs::public_platform_xml_mutators_have_closed_pre_side_effect_format_refusal
  - crates/unica-coder/src/infrastructure/format_guard.rs::dcs_edit_blocks_old_external_source_set_via_owner_descriptor
  - crates/unica-coder/src/infrastructure/format_guard.rs::cf_init_public_guard_blocks_newer_existing_post_validation_dependency
supersedes: []
superseded-by: null
establishes: [INV.SOURCE.WRITABLE-PROFILE, INV.SOURCE.OWNER-VERSION-GATE, INV.SOURCE.NO-FORMAT-MIGRATION]
---

# Записываемый профиль platform XML остаётся единственным

**Решение.** Единственный профиль, в который Unica пишет platform XML, —
платформа `8.3.27`, формат `2.20`. Версию определяет точный корень-владелец:
подчинённый документ наследует его свидетельство, безопасное чтение старшего
или младшего формата предупреждает, а запись вне профиля отказывает до первого
байта. Отсутствующая версия известного корня-владельца означает `1.0`, но
отсутствующий, неизвестный или неоднозначный владелец этим значением не
подменяется.

Нативной миграции и параметра целевого формата нет: перенос выполняется самой
платформой с последующей выгрузкой. Второй профиль требует нового
продуктового решения, отдельного корпуса и полного независимого гейта, а не
добавления ветви или значения параметра в это решение.

При замещении `ADR-0016` его составное правило сужается: самостоятельная норма
platform-before-XSD не переносится в v2, потому что в отслеживаемом репозитории
нет сохранённой независимой пары выгрузок платформы `8.3.27` до и после
кругового импорта. Сохраняются только проверенные обязательства единственного
записываемого профиля, точного корня-владельца, отказа от нативной миграции и
нового решения до появления второго профиля. Порядок источников истины для XML
остаётся задан правилами репозитория: спецификация и доказанные фикстурами
эмиттер или выгрузка платформы старше `arch/`; это пояснение не создаёт нового
проверяемого инварианта.

**Почему.** Способность платформы импортировать другой формат не доказывает,
что прямое редактирование такого дерева безопасно.

**Цена.** Пользователь переносит старую выгрузку явно, а поддержка следующей
линейки платформы не появляется постепенно или неявно.

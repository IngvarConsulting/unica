---
id: INV.SOURCE.OWNER-VERSION-GATE
status: active
governs: product
decision: DEC.2026-08-21.SINGLE-WRITABLE-PLATFORM-XML-PROFILE
check:
  - crates/unica-coder/src/infrastructure/format_guard.rs::newer_dump_warns_for_read_only_with_roadmap_copy
  - crates/unica-coder/src/infrastructure/format_guard.rs::mxl_info_warns_old_external_source_set_via_owner_descriptor
  - crates/unica-coder/src/infrastructure/format_guard.rs::missing_root_version_is_classified_as_1_0
  - crates/unica-coder/src/infrastructure/format_guard.rs::versionless_known_standalone_form_is_classified_as_1_0_owner
  - crates/unica-coder/src/infrastructure/format_guard.rs::dcs_edit_blocks_old_external_source_set_via_owner_descriptor
  - crates/unica-coder/src/infrastructure/format_guard.rs::version_owning_target_cannot_hide_behind_supported_source_set_owner
  - crates/unica-coder/src/infrastructure/format_guard.rs::xdto_guard_empty_handler_resolution_is_a_contract_error
  - crates/unica-coder/src/infrastructure/format_guard.rs::valid_standalone_mxl_without_owner_version_is_not_an_old_dump
  - crates/unica-coder/src/infrastructure/format_guard.rs::unknown_version_bearing_roots_are_rejected_by_the_closed_policy_catalog
  - crates/unica-coder/src/infrastructure/platform_xml_owner.rs::equal_depth_source_set_owners_are_ambiguous_for_existing_and_new_outputs
scope: [source]
---

# Версию решает корень-владелец до первой записи

Подчинённый XML наследует формат точного корня-владельца. Более новая выгрузка
предупреждает при безопасном чтении, старая и отсутствующая версия известного
владельца классифицируются точно, запись в старый внешний набор блокируется, а
известный DCS или MXL без собственной версии не получает выдуманного владельца.
Неизвестный версионированный корень отказывает закрыто.

---
id: DEC.2026-09-03.INLINE-COMMAND-REGISTRATION
status: active
governs: product
realized:
  - crates/unica-coder/src/infrastructure/source_revision.rs::retained_snapshot_rejects_a_capability_from_another_source_identity
  - crates/unica-coder/src/infrastructure/source_revision.rs::retained_snapshot_reuses_a_clean_fence_and_reconciles_once_after_change
  - crates/unica-coder/src/infrastructure/source_revision.rs::retained_manifest_uses_the_existing_source_digest_algorithm
  - crates/unica-coder/src/infrastructure/source_revision.rs::review_retained_manifest_cannot_satisfy_an_ambient_fast_path_after_root_swap
  - crates/unica-coder/src/infrastructure/source_revision.rs::unsupported_fence_stable_operation_lease_scans_at_admission_and_confirmation
  - crates/unica-coder/src/infrastructure/source_revision.rs::unsupported_fence_reconcile_is_bounded_to_six_passes_when_corpus_never_stabilizes
  - crates/unica-coder/src/infrastructure/source_revision.rs::review_final_confirmation_rejects_root_replacement_during_retained_scan
  - crates/unica-coder/src/infrastructure/source_revision.rs::review_final_confirmation_rejects_nested_directory_replacement_after_retention
  - crates/unica-coder/src/infrastructure/source_revision.rs::review_final_confirmation_rejects_file_replacement_after_retention
  - crates/unica-coder/src/infrastructure/source_revision.rs::review_final_confirmation_rejects_membership_added_after_enumeration
  - crates/unica-coder/src/infrastructure/source_revision.rs::review_final_confirmation_rejects_in_place_change_after_hash
  - crates/unica-coder/src/infrastructure/source_revision.rs::retained_scan_limits_entries_files_and_aggregate_bytes
  - crates/unica-coder/src/infrastructure/source_revision.rs::retained_file_hashing_checks_cancellation_between_bounded_chunks
  - crates/unica-coder/src/infrastructure/source_revision.rs::retained_snapshot_never_mixes_a_replaced_root_name_with_the_open_tree
  - crates/unica-coder/src/infrastructure/source_revision.rs::ambient_manifest_cannot_satisfy_a_retained_fast_path
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::object_commands_are_registered_inline_without_descriptor_files
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::template_bodies_are_read_only_when_the_template_node_is_addressed
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::add_in_templates_stop_addressing_at_the_template_without_reading_the_payload
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::configuration_level_rights_are_readable_role_objects
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::actor_owned_reader_never_follows_a_source_set_remap_after_admission
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::actor_owned_configuration_support_and_home_page_sidecars_are_retained
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::actor_owned_typed_form_reader_never_follows_a_source_set_remap
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::actor_owned_module_reader_never_follows_a_source_set_remap
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::every_typed_reader_remains_on_the_admitted_root_after_source_set_remap
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::actor_supplied_extension_kind_preserves_extension_support_semantics
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::configuration_root_branch_counts_match_every_reachable_collection
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::module_capability_parents_expose_canonical_module_collections
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::configuration_runtime_modules_are_read_from_the_shared_ext_layout
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::every_accepted_profile_address_has_a_real_non_skipping_view
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::real_typed_readers_cover_every_task14_profile_without_skipping
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::every_reader_rejects_an_extra_unconsumed_address_tail
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::form_table_column_event_consumes_arbitrary_depth_and_preserves_owner_address
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::form_projection_uses_a_positive_nested_scalar_allowlist
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::role_merges_access_by_canonical_object_and_keeps_rls_under_that_right
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::role_right_projection_never_serializes_an_unbounded_rights_array_into_props
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::retained_home_page_distinguishes_missing_from_malformed_and_wrong_root
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::unsupported_view_filter_is_a_typed_bad_value_instead_of_a_noop
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::module_body_context_filter_excludes_at_client_source_from_server_slice
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::module_method_public_filter_returns_only_export_methods
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::typed_projection_never_leaks_provider_or_physical_slots
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::typed_projection_rejects_unknown_provider_payload_instead_of_dumping_it
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::missing_owner_module_branch_is_not_invented_but_registered_owner_without_bsl_is_kept
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::real_external_sources_are_traversable_without_configuration_xml_and_hide_root_runtime_modules
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::external_inventory_skips_runtime_sidecar_and_fails_closed_on_malformed_or_ambiguous_owner
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::retained_external_inventory_is_cancellable_and_has_an_aggregate_byte_bound
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::production_authorities_reach_all_profile_module_capabilities_from_real_parent_inventories
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::ambiguous_short_role_alias_is_rejected_and_canonical_aliases_work
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::review_role_canonical_encoding_cannot_collapse_distinct_kind_name_pairs
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::review_rejects_direct_typed_owner_absent_from_configuration_inventory
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::review_rejects_orphan_nested_module_owners_not_registered_by_parent
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::registered_physical_child_with_wrong_descriptor_fails_direct_and_parent_navigation
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::registered_top_level_owner_without_descriptor_fails_kind_branch_and_direct_view
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::orphan_and_missing_physical_children_fail_closed_across_reader_families
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::external_parent_childobjects_are_the_only_nested_owner_authority
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::unregistered_top_level_descriptors_cannot_enter_any_typed_reader_family
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::review_revision_authority_cannot_be_swapped_after_named_identity_validation
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::operation_lease_rejects_named_root_replacement_before_node_read
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::review_rejects_revision_change_during_post_fence_owner_proof
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::cursor_retry_rejects_revision_change_during_role_canonicalization
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::review_production_read_port_has_no_nocancel_inventory_entrypoint
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::review_role_rejects_non_platform_metadata_node_kinds
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::operation_lease_find_traversal_scans_once_then_confirms_once
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::websocket_client_source_view_is_an_explicit_provider_gap
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::extension_platform_event_does_not_advertise_unproved_interception
  - crates/unica-coder/src/infrastructure/v13_read/tests.rs::extension_root_platform_modules_are_owned_by_the_extension_root
supersedes: []
superseded-by: null
establishes: [INV.SOURCE.LOGICAL-READER-PARITY]
design: docs/design/2026-09-03-inline-command-registration-design.md
---

# Команда владельца доказывается инлайн-регистрацией, а не файлом дескриптора

**Решение.** Регистрация дочернего объекта в `ChildObjects` владельца — это
текст элемента, а при его отсутствии `Properties/Name` вложенного
определения. Форма и макет остаются текстовой ссылкой с обязательным
matching descriptor. Команда регистрируется своим полным инлайн-определением
`<Command uuid="…"><Properties><Name>…`, файла `Commands/<Имя>.xml` не имеет,
и доказательство владельца для пары `Command.<Имя>` состоит из этой
регистрации; вложенных физических пар после команды нет. Модуль команды
читается по прежнему пути `Commands/<Имя>/Ext/CommandModule.bsl`, а
незарегистрированный каталог команды остаётся неадресуемым.

**Почему.** Так пишет платформа 8.3.27 и так описывает формат
`plugins/unica/references/specs/1c-config-objects-spec.md`; дамп УТ не
содержит ни одного дескриптора команды при 567 модулях команд, и любой
владелец с командами проваливал owner admission в `view` и `find`.

**Цена.** Прежние фикстуры v13 с отдельными дескрипторами команд заменены на
форму платформы; проверка неправильного дочернего дескриптора закреплена на
форме.

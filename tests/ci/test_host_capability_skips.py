"""Тест без возможности хоста виден пропуском, а не PASS (#1893).

Тест, которому нужна возможность хоста (симлинки, регистронезависимый том,
биты прав Unix), сообщает о её отсутствии через
`unica_process::test_support::skip_without_host_capability` и сразу выходит.
Раннер превращает такой PASS в пропуск с причиной и отвергает его на раннере,
где возможность обязательна (`scripts/ci/ci_rust_lanes.py`). Тест, которому
нужна возможность конкретной ОС, помечается
`#[cfg_attr(not(<ОС>), ignore = "host capability ...")]` там, где граница
платформы допускает OS-cfg (`tests/platform/`, фасады `src/platform/`);
в остальных местах — helper с именем возможности ОС.

Страж ищет ранний выход рядом с пробой возможности в обход helper. Места,
найденные до механизма, перечислены ниже. Их переводят пакетами по крейтам,
и каждый пакет удаляет свои строки: устаревшая строка роняет страж так же,
как новое место.
"""
from __future__ import annotations

import unittest

import host_capability_guard as guard

# Ранние выходы, существовавшие до механизма (#1893). Список только сокращается.
ALLOWLIST = frozenset({
    "crates/unica-adapter-rlm/src/generation_collection.rs::a_link_named_like_a_generation_is_neither_followed_nor_removed",
    "crates/unica-adapter-rlm/src/generation_collection.rs::a_linked_generation_parent_is_not_walked",
    "crates/unica-adapter-rlm/src/generation_collection.rs::a_recent_file_deep_in_a_generation_keeps_it",
    "crates/unica-adapter-rlm/src/generation_collection.rs::content_below_the_walk_limit_keeps_the_generation",
    "crates/unica-adapter-rlm/tests/platform/index.rs::active_build_collects_an_idle_previous_generation_but_not_the_current_one",
    "crates/unica-adapter-rlm/tests/platform/index.rs::builder_15_rejects_a_generation_alias_to_builder_14_storage",
    "crates/unica-adapter-rlm/tests/platform/index.rs::builder_command_preserves_native_source_root_argument_bytes",
    "crates/unica-adapter-rlm/tests/platform/index.rs::builder_generation_environment_preserves_native_path_bytes",
    "crates/unica-adapter-rlm/tests/platform/index.rs::candidate_symlink_route_never_replaces_last_usable_marker",
    "crates/unica-adapter-rlm/tests/platform/index.rs::path_normalization_failures_do_not_match_index_identity",
    "crates/unica-adapter-rlm/tests/platform/index.rs::rlm_provider_state_identity_preserves_distinct_non_utf8_unix_paths",
    "crates/unica-adapter-rlm/tests/platform/index.rs::rlm_provider_state_identity_preserves_unix_path_case",
    "crates/unica-adapter-rlm/tests/platform/index.rs::safe_root_failure_is_unavailable_at_the_start_boundary",
    "crates/unica-coder/src/application/mod.rs::cf_edit_preserves_unix_mode_0600",
    "crates/unica-coder/src/application/mod.rs::cf_edit_rejects_symlink_configuration_without_touching_referent",
    "crates/unica-coder/src/infrastructure/capacity_observation.rs::report_reader_rejects_linked_snapshot_and_storage_failure_is_fail_open",
    "crates/unica-coder/src/infrastructure/capacity_observation.rs::report_reader_rejects_symlink_without_following_it",
    "crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_dcs_check_refuses_linked_external_payload",
    "crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_view_bootstrap_classifies_broken_project_config_link_as_invalid",
    "crates/unica-coder/src/infrastructure/documentation_policy_integration.rs::dangling_policy_config_links_fail_closed_as_present_files",
    "crates/unica-coder/src/infrastructure/internal_adapters.rs::bounded_external_epf_rejects_case_only_output_aliases",
    "crates/unica-coder/src/infrastructure/metadata_operations.rs::child_collection_snapshot_failure_is_provider_neutral",
    "crates/unica-coder/src/infrastructure/metadata_operations.rs::child_topology_failures_report_logical_identity_without_provider_paths",
    "crates/unica-coder/src/infrastructure/metadata_operations.rs::typed_event_source_rejects_dependency_ancestor_symlink_after_prepare",
    "crates/unica-coder/src/infrastructure/metadata_operations.rs::typed_event_source_rejects_handler_module_ancestor_symlink_after_prepare",
    "crates/unica-coder/src/infrastructure/native_operations/apply_families/metadata.rs::command_lifecycle_refuses_links_at_the_source_or_destination_module_route",
    "crates/unica-coder/src/infrastructure/native_operations/external.rs::normalized_output_path_never_traverses_symlink_before_parent_component",
    "crates/unica-coder/src/infrastructure/native_operations/logical_selector.rs::logical_selector_refuses_a_symlinked_resource",
    "crates/unica-coder/src/infrastructure/native_operations/logical_selector.rs::physical_support_target_accepts_regular_and_verbatim_windows_paths",
    "crates/unica-coder/src/infrastructure/native_operations/logical_selector.rs::physical_support_target_rejects_a_parent_directory_link_or_reparse_point",
    "crates/unica-coder/src/infrastructure/native_operations/role.rs::rights_drift_in_the_staging_window_is_classified_as_concurrent",
    "crates/unica-coder/src/infrastructure/native_operations/role.rs::role_resource_symlink_swap_after_planning_is_rejected_before_publication",
    "crates/unica-coder/src/infrastructure/native_operations/role.rs::symlinked_role_resource_is_rejected_without_following_it",
    "crates/unica-coder/src/infrastructure/native_operations/xdto.rs::xdto_guard_rejects_resource_outside_selected_source_set",
    "crates/unica-coder/src/infrastructure/operational_config.rs::dangling_config_link_fails_closed_as_an_unreadable_present_layer",
    "crates/unica-coder/src/infrastructure/platform_xml_owner.rs::metadata_wrapper_resolution_rejects_symlink_without_following_target",
    "crates/unica-coder/src/infrastructure/platform_xml_owner.rs::require_regular_owner_rejects_symlink_without_following_target",
    "crates/unica-coder/src/infrastructure/platform_xml_owner.rs::source_set_owner_resolution_rejects_symlink_without_following_target",
    "crates/unica-coder/src/infrastructure/platform_xml_owner.rs::standalone_owner_resolution_rejects_symlink_without_following_target",
    "crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs::platform_xml_object_target_refuses_a_linked_descriptor",
    "crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs::platform_xml_source_targets_reject_a_configured_source_root_symlink",
    "crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs::platform_xml_source_targets_reject_a_source_root_replaced_by_a_symlink",
    "crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs::platform_xml_source_targets_reject_symlinked_layout_ancestor",
    "crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs::platform_xml_source_targets_reject_symlinked_target",
    "crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs::platform_xml_source_targets_revalidation_rejects_a_replaced_symlink_target",
    "crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs::source_set_relative_path_does_not_follow_a_replaced_named_root",
    "crates/unica-coder/src/infrastructure/runtime_jobs_integration.rs::a_broken_record_link_is_not_proof_that_a_record_is_absent",
    "crates/unica-coder/src/infrastructure/runtime_jobs_integration.rs::a_linked_job_directory_is_not_proof_that_a_record_is_absent",
    "crates/unica-coder/src/infrastructure/runtime_jobs_integration.rs::symlinked_jobs_directory_between_lifecycle_lock_and_admission_starts_no_producer",
    "crates/unica-coder/src/infrastructure/runtime_preflight_integration.rs::build_preflight_rejects_a_linked_local_config_overlay",
    "crates/unica-coder/src/infrastructure/source_roots.rs::declared_source_root_reports_link_before_normalized_containment",
    "crates/unica-coder/src/infrastructure/source_roots.rs::rejects_nonexistent_source_dir_through_symlink_outside_workspace",
    "crates/unica-coder/src/infrastructure/support_state.rs::platform_xml_support_reader_rejects_marker_below_linked_ext_directory",
    "crates/unica-coder/src/infrastructure/tool_context.rs::runtime_stderr_output_rejects_symlink_workspace_escape",
    "crates/unica-coder/src/infrastructure/v13_find.rs::a_linked_common_module_alias_refuses_the_layout",
    "crates/unica-coder/src/infrastructure/v13_find.rs::absolute_lookup_accepts_a_physical_unicode_alias_of_the_source_root",
    "crates/unica-coder/src/infrastructure/v13_find.rs::absolute_lookup_distinguishes_a_file_from_a_case_folded_foreign_root",
    "crates/unica-coder/src/infrastructure/v13_find.rs::absolute_lookup_does_not_open_a_damaged_case_distinct_foreign_parent",
    "crates/unica-coder/src/infrastructure/v13_find.rs::absolute_lookup_refuses_a_replaced_requested_root_ancestor",
    "crates/unica-coder/src/infrastructure/v13_find.rs::linked_command_directory_refuses_instead_of_looking_complete",
    "crates/unica-coder/src/infrastructure/v13_find.rs::linked_configuration_descriptor_refuses_the_layout_build",
    "crates/unica-coder/src/infrastructure/v13_find.rs::linked_nested_descriptor_refuses_instead_of_looking_complete",
    "crates/unica-coder/src/infrastructure/v13_find.rs::linked_proven_owner_directory_refuses_instead_of_hiding_nested_names",
    "crates/unica-coder/src/infrastructure/v13_find.rs::point_lookup_nested_path_ignores_an_unrelated_linked_owner_directory",
    "crates/unica-coder/src/infrastructure/v13_find.rs::point_lookup_refuses_a_linked_target_descriptor",
    "crates/unica-coder/src/infrastructure/v13_find.rs::relative_source_prefix_skips_linked_collections_in_another_source",
    "crates/unica-coder/src/infrastructure/v13_find.rs::windows_drive_and_unc_prefixes_keep_the_same_root_anchor_shape",
    "crates/unica-coder/src/infrastructure/v13_read/tests.rs::canonical_dcs_validation_does_not_follow_links_after_input_capture",
    "crates/unica-coder/src/infrastructure/v13_read/tests.rs::canonical_dcs_validation_refuses_linked_format_owners_before_capture",
    "crates/unica-coder/src/infrastructure/v13_read/tests.rs::canonical_subsystem_check_refuses_linked_dependencies",
    "crates/unica-coder/src/infrastructure/v13_read/tests.rs::canonical_subsystem_check_refuses_replaced_root",
    "crates/unica-coder/src/infrastructure/v13_read/tests.rs::diagnostic_mapping_refuses_replacement_of_its_retained_source_root",
    "crates/unica-coder/src/infrastructure/v13_read/tests.rs::diagnostic_module_scope_refuses_module_replaced_by_foreign_symlink",
    "crates/unica-coder/src/infrastructure/v13_read/tests.rs::diagnostic_module_scope_refuses_replaced_retained_root",
    "crates/unica-coder/src/infrastructure/v13_read/tests.rs::named_external_view_rejects_replaced_retained_root_without_reading_ambient_owner",
    "crates/unica-coder/src/infrastructure/v13_read/tests.rs::read_authority_rejects_named_root_replacement_before_node_read",
    "crates/unica-coder/src/infrastructure/v13_read/tests/form.rs::form_metadata_rejects_orphans_and_does_not_follow_descriptor_links",
    "crates/unica-coder/src/infrastructure/workspace_metadata_publication_tests.rs::command_module_lifecycle_late_case_rename_rolls_back_owned_files_and_cache",
    "crates/unica-coder/src/interfaces/mcp.rs::public_native_cancel_waits_for_slow_job_attach_before_answering",
    "crates/unica-coder/tests/platform/code_intelligence_integration.rs::git_grep_scoped_preflight_refuses_invalid_routes_without_dispatch",
    "crates/unica-coder/tests/platform/code_patch_composition.rs::code_patch_symlink_module_is_rejected_during_preview",
    "crates/unica-coder/tests/platform/receipt_runtime.rs::displaced_runtime_cannot_write_a_response_after_the_final_authority_check",
    "crates/unica-coder/tests/platform/receipt_runtime.rs::displaced_runtime_retains_stable_authority_until_the_old_owner_drops",
    "crates/unica-coder/tests/platform/receipt_runtime.rs::replacement_receipt_authority_directory_alone_cannot_create_a_successor_runtime",
    "crates/unica-coder/tests/platform/subsystem_subsystem_info_typed_result_tests.rs::bak_symlink_alias_is_not_a_subsystem_info_xml_target",
    "crates/unica-coder/tests/platform/subsystem_subsystem_info_typed_result_tests.rs::nested_subsystems_symlink_is_not_followed_for_a_tree_answer",
    "crates/unica-coder/tests/platform/subsystem_subsystem_info_typed_result_tests.rs::parent_component_before_subsystems_is_rejected_before_any_registered_read",
    "crates/unica-coder/tests/platform/subsystem_subsystem_info_typed_result_tests.rs::root_subsystems_symlink_is_not_followed_for_a_tree_answer",
    "crates/unica-coder/tests/platform/subsystem_subsystem_info_typed_result_tests.rs::xml_beneath_an_intermediate_directory_symlink_is_not_a_local_target",
    "crates/unica-coder/tests/platform/subsystem_subsystem_info_typed_result_tests.rs::xml_symlink_alias_is_not_a_subsystem_info_target",
    "crates/unica-coder/tests/platform/workspace_services_integration.rs::bsl_analyzer_runtime_directory_is_passed_to_the_child_and_fits_socket_budget",
    "crates/unica-coder/tests/platform/workspace_services_integration.rs::invalidated_analyzer_preserves_its_owner_after_exit_observation_is_lost",
    "crates/unica-coder/tests/platform/workspace_services_integration.rs::provider_state_key_keeps_distinct_non_utf8_path_identities_separate",
    "crates/unica-coder/tests/platform/workspace_services_integration.rs::rlm_execute_rejects_replaced_root_and_retires_its_session",
    "crates/unica-coder/tests/platform/workspace_services_integration.rs::rlm_reader_command_preserves_native_generation_environment_bytes",
    "crates/unica-coder/tests/platform/workspace_services_integration.rs::rlm_ready_rejects_unsafe_provider_state_root",
    "crates/unica-execution/src/lifecycle_link_store.rs::second_pass_refuses_link_staging_symlink_and_preserves_external_bytes",
    "crates/unica-execution/src/receipt_store/tests.rs::authority_failure_latches_every_live_entry_point_until_reopen",
    "crates/unica-execution/src/receipt_store/tests.rs::cancel_at_maximum_epoch_cannot_bypass_an_already_fail_stopped_store",
    "crates/unica-execution/src/receipt_store/tests.rs::failed_prepublication_cleanup_fail_stops_the_store_until_reopen",
    "crates/unica-execution/src/receipt_store/tests.rs::observed_receipt_acl_drift_latches_the_writer_before_another_reservation",
    "crates/unica-execution/src/receipt_store/tests.rs::receipt_directory_link_or_reparse_point_is_rejected_without_touching_target",
    "crates/unica-execution/src/receipt_store/tests.rs::recovery_cleanup_preserves_external_targets_of_replaced_active_and_generation_orphans",
    "crates/unica-execution/src/task_store.rs::second_pass_refuses_task_staging_symlink_and_preserves_external_bytes",
    "crates/unica-project-health/tests/platform/project_health_git.rs::project_health_git_discovers_repository_root_containing_newline",
    "crates/unica-project-health/tests/platform/project_health_git.rs::project_health_git_ignore_rejects_host_identity_collisions",
    "crates/unica-project-health/tests/platform/project_health_inspection.rs::source_owner_index_applies_case_policy_per_parent_directory",
    "crates/unica-project-health/tests/platform/project_health_inspection.rs::source_owner_index_uses_the_host_volume_case_policy",
    "crates/unica-project-health/tests/platform/project_health_layout.rs::linked_alias_to_workspace_reports_the_primary_identity_cause_with_evidence",
    "crates/unica-project-health/tests/platform/project_health_layout.rs::linked_route_to_another_nested_root_is_unsafe",
    "crates/unica-project-health/tests/platform/project_health_layout.rs::linked_route_without_trusted_format_does_not_pass_format_or_generated_checks",
    "crates/unica-project-health/tests/platform/project_health_layout.rs::unreadable_source_root_is_not_ready_for_source_operations",
    "crates/unica-project-health/tests/platform/project_health_resources.rs::project_health_lfs_size_does_not_follow_linked_parent_directory",
    "crates/unica-project-health/tests/platform/project_health_resources.rs::project_health_lfs_size_failure_is_not_run_but_advisory",
    "crates/unica-project-health/tests/platform/project_health_resources.rs::project_health_worktree_eol_rejects_a_symlink_at_open",
    "crates/unica-source-integration-tests/tests/project_sources.rs::autodetect_keeps_a_non_utf8_extension_name_from_hiding_its_siblings",
    "crates/unica-source-integration-tests/tests/project_sources.rs::autodetect_propagates_an_unreadable_extensions_container",
    "crates/unica-source-integration-tests/tests/project_sources.rs::autodetect_skips_a_linked_extensions_container",
    "crates/unica-source-integration-tests/tests/project_sources.rs::autodetect_skips_linked_entries_in_the_extensions_container",
    "crates/unica-source-integration-tests/tests/project_sources.rs::health_autodetect_does_not_follow_linked_marker_parent",
    "crates/unica-source-integration-tests/tests/project_sources.rs::health_source_map_does_not_probe_through_linked_marker_parent",
    "crates/unica-source-integration-tests/tests/project_sources.rs::health_source_map_does_not_probe_through_linked_source_root",
    "crates/unica-source-integration-tests/tests/project_sources.rs::symlinked_config_dump_info_is_not_external_source_format_evidence",
    "crates/unica-source-integration-tests/tests/project_sources.rs::transactional_autodetect_binds_a_linked_container_path",
    "crates/unica-source-integration-tests/tests/project_sources.rs::transactional_autodetect_tolerates_a_link_beside_the_extensions",
    "crates/unica-workspace/src/actor_local_tests.rs::actor_state_scope_distinguishes_native_non_unicode_paths",
    "crates/unica-workspace/src/actor_local_tests.rs::descriptor_relative_read_never_follows_a_nested_directory_link",
    "crates/unica-workspace/src/actor_local_tests.rs::retained_binding_rejects_a_root_replaced_by_another_source_set_link",
    "crates/unica-workspace/src/actor_policy_contract_tests.rs::apply_policy_stable_deny_evidence_allows_unrelated_dry_run_and_real_publication",
    "crates/unica-workspace/src/actor_policy_contract_tests.rs::cache_preimage_link_race_and_absent_chain_occupation_preserve_foreign_namespace",
    "crates/unica-workspace/src/actor_policy_contract_tests.rs::prepared_apply_cleanup_race_surfaces_a_relative_actor_diagnostic",
    "crates/unica-workspace/src/actor_policy_contract_tests.rs::source_role_rejects_platform_equivalent_generated_components",
    "crates/unica-workspace/src/apply.rs::retained_create_stage_source_swap_leaves_no_destination_mutation_and_loses_neither_file",
    "crates/unica-workspace/src/apply.rs::retained_directory_rollback_never_unlinks_a_public_name_swapped_after_validation",
    "crates/unica-workspace/src/apply.rs::retained_private_directory_source_swap_never_publishes_or_deletes_the_foreign_directory",
    "crates/unica-workspace/src/apply.rs::retained_rollback_published_cleanup_never_unlinks_a_concurrent_child",
    "crates/unica-workspace/src/apply.rs::retained_rollback_restore_recovery_swap_never_reports_a_different_inode_restored",
    "crates/unica-workspace/src/apply.rs::retained_success_recovery_cleanup_never_unlinks_a_concurrent_child",
    "crates/unica-workspace/src/compile_transaction_contract_tests.rs::pending_registration_cleanup_preserves_same_name_decoys_after_parent_swap",
    "crates/unica-workspace/src/compile_transaction_contract_tests.rs::post_validation_rollback_restores_bytes_and_unix_mode_0600",
    "crates/unica-workspace/src/compile_transaction_contract_tests.rs::registration_rollback_preserves_same_name_recovery_decoy_after_parent_swap",
    "crates/unica-workspace/src/compile_transaction_contract_tests.rs::registration_rollback_restore_uses_retained_parent_handles_after_route_swap",
    "crates/unica-workspace/src/compile_transaction_contract_tests.rs::successful_registration_cleanup_warns_and_preserves_decoy_after_parent_swap",
    "crates/unica-workspace/src/compile_transaction_contract_tests.rs::symlink_targets_are_rejected",
    "crates/unica-workspace/src/compile_transaction_local_tests.rs::publication_cleanup_retry_does_not_treat_a_missing_route_as_cleaned",
    "crates/unica-workspace/src/compile_transaction_local_tests.rs::replacement_rejects_parent_symlink_swap_after_planning_before_locking",
    "crates/unica-workspace/src/compile_transaction_local_tests.rs::transaction_rejects_cache_create_parent_symlink_swap_before_touching_either_target",
    "crates/unica-workspace/src/platform/filesystem.rs::apple_caseless_child_identity_uses_the_filesystem_object",
    "crates/unica-workspace/src/platform/filesystem.rs::secure_absolute_directory_creation_rejects_link_ancestor_without_writing_through_it",
    "crates/unica-workspace/src/platform/filesystem.rs::windows_open_any_child_nofollow_classifies_a_directory_reparse_point",
    "crates/unica-workspace/src/policy_evidence.rs::retained_support_policy_candidate_parent_replacement_is_rejected",
    "crates/unica-workspace/src/policy_evidence.rs::retained_support_policy_exact_and_oversized_reject_name_replacement_after_pre_read_identity",
    "crates/unica-workspace/src/policy_evidence.rs::retained_support_policy_exact_rejects_name_replacement_after_retained_read_before_acceptance",
    "crates/unica-workspace/src/single_file_publisher.rs::bound_cleanup_cannot_be_redirected_after_the_last_route_check",
    "crates/unica-workspace/src/single_file_publisher.rs::link_or_reparse_target_is_rejected_without_touching_referent",
    "crates/unica-workspace/src/single_file_publisher.rs::lock_only_identity_rejects_an_existing_symlink_ancestor",
    "crates/unica-workspace/src/single_file_publisher.rs::permission_change_before_commit_is_rejected",
    "crates/unica-workspace/src/single_file_publisher.rs::sync_failpoint_reaches_post_permissions_checkpoint",
    "crates/unica-workspace/src/workspace_actor_cache_tests.rs::cache_override_drive_prefix_follows_host_path_semantics_without_erasing_links",
    "crates/unica-workspace/src/workspace_actor_cache_tests.rs::external_cache_route_rejects_links_including_components_cancelled_by_parent",
    "crates/unica-workspace/src/workspace_actor_cache_tests.rs::relative_cache_uses_process_cwd_when_workspace_has_an_alias",
    "crates/unica-workspace/src/workspace_actor_cache_tests.rs::shared_workspace_alias_does_not_retarget_prepared_external_or_default_cache",
    "crates/unica-workspace/src/workspace_actor_cache_tests.rs::workspace_suffix_alias_cannot_disguise_a_different_shared_cache_ancestor",
})


class HostCapabilityGuardTests(unittest.TestCase):
    def test_no_new_early_exit_and_no_stale_allowlist_entry(self) -> None:
        found = guard.repository_offenses()
        self.assertEqual(sorted(found - ALLOWLIST), [], "test exits early on a host capability without the skip helper")
        self.assertEqual(sorted(ALLOWLIST - found), [], "converted tests must leave the allowlist")

    def test_detects_capability_exits_in_every_shape(self) -> None:
        shapes = {
            "let_else": "let Some(link) = create_file_symlink_for_test(&a, &b) else { return; };",
            "if_not": "if !can_swap_named_child_behind_retained_handle_for_test() {\n    return;\n}",
            "match_arm": "match outcome {\n    FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,\n    _ => {}\n}",
            "raw_error": "if error.raw_os_error() == Some(1314) { return; }",
            "raw_symlink": "if std::os::unix::fs::symlink(&a, &b).is_err() { eprintln!(\"skip\"); return; }",
            "loop": "for case in cases {\n    if host_filesystem_case_sensitive(&root).unwrap() { continue; }\n}",
            "second_exit": "let Some(l) = create_file_symlink_for_test(&a, &b) else {\n"
                           "    skip_without_host_capability(HostCapability::Symlink, \"x\");\n    return;\n};\n"
                           "if l.is_err() { return; }",
        }
        for name, statement in shapes.items():
            with self.subTest(shape=name):
                source = "#[test]\nfn sample() {\n    " + statement + "\n    assert!(true);\n}\n"
                self.assertEqual(guard.offenses(source, "crates/demo/src/lib.rs"), ["crates/demo/src/lib.rs::sample"])

    def test_the_skip_helper_comments_strings_and_production_code_are_not_offenses(self) -> None:
        allowed = (
            "#[test]\nfn sample() {\n    if !can_swap_named_child_behind_retained_handle_for_test() {\n"
            "        skip_without_host_capability(\n            HostCapability::RenameUnderOpenHandle,\n"
            "            &format!(\"needs {}\", root.display()),\n        );\n        cleanup(&context);\n        return;\n    }\n}\n",
            "#[test]\nfn sample() {\n    let Some(link) = create_file_symlink_for_test(&a, &b) else {\n"
            "        skip_without_host_capability(HostCapability::Symlink, \"needs a privilege\");\n"
            "        return;\n    };\n}\n",
            "#[test]\nfn sample() {\n    // if !supports_retained_root_replacement_test() { return; }\n    run();\n}\n",
            "#[test]\nfn sample() {\n    let program = r#\"fn main() { if symlink(a, b).is_err() { return; } }\"#;\n}\n",
            "#[test]\nfn sample() {\n    let link = create_file_symlink_for_test(&a, &b).unwrap().unwrap();\n    assert!(link);\n}\n",
        )
        for source in allowed:
            with self.subTest(source=source):
                self.assertEqual(guard.offenses(source, "crates/demo/src/lib.rs"), [])
        production = "pub fn case_policy(root: &Path) -> bool {\n    if !host_filesystem_case_sensitive(root).unwrap() { return false; }\n    true\n}\n"
        self.assertEqual(guard.offenses(production, "crates/demo/src/policy.rs"), [])
        self.assertEqual(len(guard.offenses(production, "crates/demo/src/policy_tests.rs")), 1)


if __name__ == "__main__":
    unittest.main()

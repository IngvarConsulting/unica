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
    "crates/unica-coder/src/infrastructure/native_operations/apply_families/metadata.rs::command_lifecycle_refuses_links_at_the_source_or_destination_module_route",
    "crates/unica-coder/tests/platform/code_intelligence_integration.rs::git_grep_scoped_preflight_refuses_invalid_routes_without_dispatch",
    "crates/unica-coder/tests/platform/code_patch_composition.rs::code_patch_symlink_module_is_rejected_during_preview",
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
    "crates/unica-workspace/src/apply.rs::retained_create_stage_source_swap_leaves_no_destination_mutation_and_loses_neither_file",
    "crates/unica-workspace/src/apply.rs::retained_directory_rollback_never_unlinks_a_public_name_swapped_after_validation",
    "crates/unica-workspace/src/apply.rs::retained_private_directory_source_swap_never_publishes_or_deletes_the_foreign_directory",
    "crates/unica-workspace/src/apply.rs::retained_rollback_published_cleanup_never_unlinks_a_concurrent_child",
    "crates/unica-workspace/src/apply.rs::retained_rollback_restore_recovery_swap_never_reports_a_different_inode_restored",
    "crates/unica-workspace/src/apply.rs::retained_success_recovery_cleanup_never_unlinks_a_concurrent_child",
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
            "pub fn contract() -> Result<(), CapabilitySkip> {\n    if !can_swap_named_child_behind_retained_handle_for_test() {\n"
            "        return Err(CapabilitySkip { capability: \"rename-under-open-handle\", reason: \"x\" });\n    }\n    Ok(())\n}\n",
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

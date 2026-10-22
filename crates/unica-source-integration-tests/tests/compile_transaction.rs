//! The canonical formatter is the actual CF registrar used by production.

#[test]
fn canonical_registrations_accumulate_and_preserve_source_bytes() {
    unica_workspace::compile_transaction::contract_tests::canonical_registrations_accumulate_and_preserve_source_bytes(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn canonical_registrations_accumulate_across_path_aliases() {
    unica_workspace::compile_transaction::contract_tests::canonical_registrations_accumulate_across_path_aliases(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn missing_registration_target_is_explicit_and_guards_normalized_absence() {
    unica_workspace::compile_transaction::contract_tests::missing_registration_target_is_explicit_and_guards_normalized_absence(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn missing_registration_target_appearance_rolls_back_created_outputs() {
    unica_workspace::compile_transaction::contract_tests::missing_registration_target_appearance_rolls_back_created_outputs(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn self_closing_registration_keeps_a_bom_free_lf_source_bom_free() {
    unica_workspace::compile_transaction::contract_tests::self_closing_registration_keeps_a_bom_free_lf_source_bom_free(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn appended_registration_preserves_cr_only_line_boundaries() {
    unica_workspace::compile_transaction::contract_tests::appended_registration_preserves_cr_only_line_boundaries(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn already_present_registration_commits_as_a_byte_for_byte_noop() {
    unica_workspace::compile_transaction::contract_tests::already_present_registration_commits_as_a_byte_for_byte_noop(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn existing_target_without_child_objects_is_rejected_without_mutation() {
    unica_workspace::compile_transaction::contract_tests::existing_target_without_child_objects_is_rejected_without_mutation(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn commit_creates_bom_text_and_updates_registration_once() {
    unica_workspace::compile_transaction::contract_tests::commit_creates_bom_text_and_updates_registration_once(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn after_object_files_failure_removes_files_and_created_directories() {
    unica_workspace::compile_transaction::contract_tests::after_object_files_failure_removes_files_and_created_directories(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn post_write_validation_failure_restores_exact_registration_bytes() {
    unica_workspace::compile_transaction::contract_tests::post_write_validation_failure_restores_exact_registration_bytes(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn post_validation_rollback_restores_bytes_and_unix_mode_0600() {
    if let Err(skip) = unica_workspace::compile_transaction::contract_tests::post_validation_rollback_restores_bytes_and_unix_mode_0600(unica_source_designer::registration::cf_edit_add_child_object_text) {
        unica_process::test_support::skip_named_host_capability(skip.capability, skip.reason);
    }
}

#[test]
fn registration_target_remains_present_after_backup_preparation() {
    unica_workspace::compile_transaction::contract_tests::registration_target_remains_present_after_backup_preparation(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn pending_registration_cleanup_preserves_same_name_decoys_after_parent_swap() {
    if let Err(skip) = unica_workspace::compile_transaction::contract_tests::pending_registration_cleanup_preserves_same_name_decoys_after_parent_swap(unica_source_designer::registration::cf_edit_add_child_object_text) {
        unica_process::test_support::skip_named_host_capability(skip.capability, skip.reason);
    }
}

#[test]
fn precommit_retry_finishes_the_recovery_directory_after_file_cleanup_recovers() {
    unica_workspace::compile_transaction::contract_tests::precommit_retry_finishes_the_recovery_directory_after_file_cleanup_recovers(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn successful_registration_cleanup_warns_and_preserves_decoy_after_parent_swap() {
    if let Err(skip) = unica_workspace::compile_transaction::contract_tests::successful_registration_cleanup_warns_and_preserves_decoy_after_parent_swap(unica_source_designer::registration::cf_edit_add_child_object_text) {
        unica_process::test_support::skip_named_host_capability(skip.capability, skip.reason);
    }
}

#[test]
fn successful_registration_finalize_does_not_recreate_recovery_after_directory_cleanup_failure() {
    unica_workspace::compile_transaction::contract_tests::successful_registration_finalize_does_not_recreate_recovery_after_directory_cleanup_failure(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn compile_transaction_rejects_readonly_registration_without_partial_creates() {
    unica_workspace::compile_transaction::contract_tests::compile_transaction_rejects_readonly_registration_without_partial_creates(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn compile_transaction_rejects_hard_linked_registration_without_mutation() {
    unica_workspace::compile_transaction::contract_tests::compile_transaction_rejects_hard_linked_registration_without_mutation(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn post_validation_failure_rolls_back_two_registrations_and_one_create() {
    unica_workspace::compile_transaction::contract_tests::post_validation_failure_rolls_back_two_registrations_and_one_create(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn rollback_preserves_concurrent_registration_replacement_and_original_recovery() {
    unica_workspace::compile_transaction::contract_tests::rollback_preserves_concurrent_registration_replacement_and_original_recovery(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn registration_rollback_preserves_same_name_recovery_decoy_after_parent_swap() {
    if let Err(skip) = unica_workspace::compile_transaction::contract_tests::registration_rollback_preserves_same_name_recovery_decoy_after_parent_swap(unica_source_designer::registration::cf_edit_add_child_object_text) {
        unica_process::test_support::skip_named_host_capability(skip.capability, skip.reason);
    }
}

#[test]
fn registration_rollback_cleanup_preserves_same_name_quarantine_decoy() {
    unica_workspace::compile_transaction::contract_tests::registration_rollback_cleanup_preserves_same_name_quarantine_decoy(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn registration_rollback_restore_cleanup_preserves_same_name_quarantine_decoy() {
    unica_workspace::compile_transaction::contract_tests::registration_rollback_restore_cleanup_preserves_same_name_quarantine_decoy(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn registration_rollback_validation_reports_preserved_quarantine() {
    unica_workspace::compile_transaction::contract_tests::registration_rollback_validation_reports_preserved_quarantine(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn registration_rollback_restore_uses_retained_parent_handles_after_route_swap() {
    if let Err(skip) = unica_workspace::compile_transaction::contract_tests::registration_rollback_restore_uses_retained_parent_handles_after_route_swap(unica_source_designer::registration::cf_edit_add_child_object_text) {
        unica_process::test_support::skip_named_host_capability(skip.capability, skip.reason);
    }
}

#[test]
fn after_registration_backup_failure_restores_exact_bytes_and_removes_debris() {
    unica_workspace::compile_transaction::contract_tests::after_registration_backup_failure_restores_exact_bytes_and_removes_debris(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn concurrent_registration_commits_serialize_before_preflight() {
    unica_workspace::compile_transaction::contract_tests::concurrent_registration_commits_serialize_before_preflight(unica_source_designer::registration::cf_edit_add_child_object_text);
}

#[test]
fn symlink_targets_are_rejected() {
    unica_workspace::compile_transaction::contract_tests::symlink_targets_are_rejected(
        unica_source_designer::registration::cf_edit_add_child_object_text,
    );
}

#[test]
fn reference_scan_refuses_a_file_larger_than_the_per_file_budget() {
    unica_workspace::apply::budget_contract_tests::reference_scan_refuses_a_file_larger_than_the_per_file_budget(
        unica_metadata_designer::remove::META_REMOVE_REFERENCE_FILE_MAX_BYTES,
    );
}

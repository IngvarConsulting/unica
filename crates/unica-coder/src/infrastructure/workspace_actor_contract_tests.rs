//! The real composition binds source policies, semantic planners and rendering.
use super::source_workspace::policies;
use std::path::Path;
use unica_source_contract::proposal::ApplyPlanError;
use unica_source_contract::proposal::PlannedApplyEffects;
use unica_source_contract::workspace::WorkspaceContext;
use unica_workspace::actor::{ApplyAdmission, ProviderSourceLease};
use unica_workspace::apply::ApplyStagedState;

fn plan_events(
    admission: &ApplyAdmission,
    targets: &[String],
) -> Result<(ApplyStagedState, PlannedApplyEffects), ApplyPlanError> {
    use super::native_operations::event_implementation::plan_event_implement_batch;
    use unica_bsl::profile::PlatformProfile;
    use unica_metadata::event_input::EventImplementArgs;
    let operations = targets
        .iter()
        .map(|at| EventImplementArgs {
            at: unica_address::address::QualifiedAddress::parse(at).unwrap(),
            call_type: None,
        })
        .collect::<Vec<_>>();
    let mut state = admission
        .staged_state()
        .expect("actor admission did not produce a plannable staged state");
    let effects = plan_event_implement_batch(
        &mut state.proposal(),
        "main",
        unica_source_contract::SourceSetKind::Configuration,
        PlatformProfile::v8_3_27(),
        &operations,
    )?;
    Ok((state, effects))
}
fn index_state_root(
    context: &WorkspaceContext,
    root: &Path,
    lease: Option<ProviderSourceLease>,
) -> Result<std::path::PathBuf, String> {
    let service = unica_adapter_rlm::index::WorkspaceIndexService::new(
        crate::infrastructure::analysis_composition::environment(),
    );
    match lease {
        Some(lease) => service
            .with_source_lease(lease)
            .provider_state_root_for_test(context, root),
        None => service.provider_state_root_for_test(context, root),
    }
}
fn index_active_build(
    context: &WorkspaceContext,
    root: &Path,
    lease: ProviderSourceLease,
) -> Result<(), String> {
    unica_adapter_rlm::index::WorkspaceIndexService::new(
        crate::infrastructure::analysis_composition::environment(),
    )
    .with_source_lease(lease)
    .active_build(context, root)
    .map(|_| ())
}

#[test]
fn workspace_actor_poisoned_mutation_lane_fails_closed_before_publication() {
    unica_workspace::actor::policy_contract_tests::workspace_actor_poisoned_mutation_lane_fails_closed_before_publication(policies());
}

#[test]
fn prepared_apply_lock_deadline_is_not_reclassified_by_later_cancellation() {
    unica_workspace::actor::policy_contract_tests::prepared_apply_lock_deadline_is_not_reclassified_by_later_cancellation(policies());
}

#[test]
fn capabilities_do_not_cross_distinct_actor_instances_with_equal_identity() {
    unica_workspace::actor::policy_contract_tests::capabilities_do_not_cross_distinct_actor_instances_with_equal_identity(policies());
}

#[test]
fn workspace_actor_capabilities_enforce_identity_physical_and_bounded_publication() {
    unica_workspace::actor::policy_contract_tests::workspace_actor_capabilities_enforce_identity_physical_and_bounded_publication(policies());
}

#[test]
fn borrow_dependency_revision_rejects_parent_change_since_preview() {
    unica_workspace::actor::policy_contract_tests::borrow_dependency_revision_rejects_parent_change_since_preview(policies());
}

#[test]
fn borrow_dependency_change_before_publication_refuses_preview_and_commit() {
    unica_workspace::actor::policy_contract_tests::borrow_dependency_change_before_publication_refuses_preview_and_commit(policies());
}

#[test]
fn borrow_dependency_final_preview_confirmation_detects_late_parent_change() {
    unica_workspace::actor::policy_contract_tests::borrow_dependency_final_preview_confirmation_detects_late_parent_change(policies());
}

#[test]
fn borrow_dependency_changed_after_destination_write_rolls_back_destination_only() {
    unica_workspace::actor::policy_contract_tests::borrow_dependency_changed_after_destination_write_rolls_back_destination_only(policies());
}

#[test]
fn borrow_dependency_rejects_capability_from_equal_identity_other_actor() {
    unica_workspace::actor::policy_contract_tests::borrow_dependency_rejects_capability_from_equal_identity_other_actor(policies());
}

#[test]
fn apply_plan_fence_is_targeted_and_binds_read_only_inputs() {
    unica_workspace::actor::policy_contract_tests::apply_plan_fence_is_targeted_and_binds_read_only_inputs(policies());
}

#[test]
fn disjoint_apply_plans_publish_without_a_global_revision_conflict() {
    unica_workspace::actor::policy_contract_tests::disjoint_apply_plans_publish_without_a_global_revision_conflict(policies());
}

#[test]
fn apply_admission_rejects_source_inside_cache() {
    unica_workspace::actor::policy_contract_tests::apply_admission_rejects_source_inside_cache(
        policies(),
    );
}

#[test]
fn workspace_root_source_allows_exact_generated_cache_descendant() {
    unica_workspace::actor::policy_contract_tests::workspace_root_source_allows_exact_generated_cache_descendant(policies());
}

#[test]
fn source_role_rejects_platform_equivalent_generated_components() {
    if let Err(skip) = unica_workspace::actor::policy_contract_tests::source_role_rejects_platform_equivalent_generated_components(policies()) {
        unica_process::test_support::skip_named_host_capability(skip.capability, skip.reason);
    }
}

#[test]
fn workspace_root_source_and_missing_cache_publish_through_disjoint_shared_anchor() {
    unica_workspace::actor::policy_contract_tests::workspace_root_source_and_missing_cache_publish_through_disjoint_shared_anchor(policies());
}

#[test]
fn prepared_apply_publishes_classified_binary_resource() {
    unica_workspace::actor::policy_contract_tests::prepared_apply_publishes_classified_binary_resource(policies());
}

#[test]
fn replacement_commit_cleans_its_owned_recovery() {
    unica_workspace::actor::policy_contract_tests::replacement_commit_cleans_its_owned_recovery(
        policies(),
    );
}

#[test]
fn replace_and_create_commit_cleans_the_owned_recovery() {
    unica_workspace::actor::policy_contract_tests::replace_and_create_commit_cleans_the_owned_recovery(policies());
}

#[test]
fn multiple_recoveries_across_parents_preserve_each_postimage() {
    unica_workspace::actor::policy_contract_tests::multiple_recoveries_across_parents_preserve_each_postimage(policies());
}

#[test]
fn remove_create_batch_preserves_unmodified_sibling() {
    unica_workspace::actor::policy_contract_tests::remove_create_batch_preserves_unmodified_sibling(
        policies(),
    );
}

#[test]
fn actor_revision_recovery_identity_swap_is_rejected_before_revision_install() {
    unica_workspace::actor::policy_contract_tests::actor_revision_recovery_identity_swap_is_rejected_before_revision_install(policies());
}

#[test]
fn actor_revision_recovery_hard_link_alias_is_never_discounted_or_restored() {
    unica_workspace::actor::policy_contract_tests::actor_revision_recovery_hard_link_alias_is_never_discounted_or_restored(policies());
}

#[test]
fn retained_recovery_late_failure_reaches_phase_and_rolls_back_without_receipt() {
    unica_workspace::actor::policy_contract_tests::retained_recovery_late_failure_reaches_phase_and_rolls_back_without_receipt(policies());
}

#[test]
fn unrelated_recovery_lookalikes_do_not_gain_cleanup_authority() {
    unica_workspace::actor::policy_contract_tests::unrelated_recovery_lookalikes_do_not_gain_cleanup_authority(policies());
}

#[test]
fn create_only_plan_survives_actor_restart_and_commits_exactly_once() {
    unica_workspace::actor::policy_contract_tests::create_only_plan_survives_actor_restart_and_commits_exactly_once(policies());
}

#[test]
fn retained_cleanup_failure_preserves_foreign_names_across_restart() {
    unica_workspace::actor::policy_contract_tests::retained_cleanup_failure_preserves_foreign_names_across_restart(policies());
}

#[test]
fn retained_apply_stop_causes_preserve_rollback_at_every_late_gate() {
    unica_workspace::actor::policy_contract_tests::retained_apply_stop_causes_preserve_rollback_at_every_late_gate(policies());
}

#[test]
fn actor_revision_unknown_staged_artifact_is_rejected_before_publication() {
    unica_workspace::actor::policy_contract_tests::actor_revision_unknown_staged_artifact_is_rejected_before_publication(policies());
}

#[test]
fn actor_revision_lookalike_resource_is_rejected_before_publication() {
    unica_workspace::actor::policy_contract_tests::actor_revision_lookalike_resource_is_rejected_before_publication(policies());
}

#[test]
fn actor_revision_late_failure_rolls_back_targeted_resource_without_receipt() {
    unica_workspace::actor::policy_contract_tests::actor_revision_late_failure_rolls_back_targeted_resource_without_receipt(policies());
}

#[test]
fn actor_platform_commit_accepts_a_classified_binary_resource() {
    unica_workspace::actor::policy_contract_tests::actor_platform_commit_accepts_a_classified_binary_resource(policies());
}

#[test]
fn prepared_apply_observer_sees_source_eager_cache_and_state_marker_order() {
    unica_workspace::actor::policy_contract_tests::prepared_apply_observer_sees_source_eager_cache_and_state_marker_order(policies());
}

#[test]
fn prepared_apply_success_publishes_source_cache_and_state_without_revision_record() {
    unica_workspace::actor::policy_contract_tests::prepared_apply_success_publishes_source_cache_and_state_without_revision_record(policies());
}

#[test]
fn retained_apply_failures_restore_source_cache_and_revision_machine_exactly() {
    unica_workspace::actor::policy_contract_tests::retained_apply_failures_restore_source_cache_and_revision_machine_exactly(policies());
}

#[test]
fn unchanged_selection_input_replaced_after_writes_refuses_and_rolls_back() {
    unica_workspace::actor::policy_contract_tests::unchanged_selection_input_replaced_after_writes_refuses_and_rolls_back(policies());
}

#[test]
fn retained_apply_final_cancellation_gate_rolls_back_all_participants() {
    unica_workspace::actor::policy_contract_tests::retained_apply_final_cancellation_gate_rolls_back_all_participants(policies());
}

#[test]
fn retained_apply_late_deadline_after_all_writes_rolls_back_all_participants() {
    unica_workspace::actor::policy_contract_tests::retained_apply_late_deadline_after_all_writes_rolls_back_all_participants(policies());
}

#[test]
fn retained_apply_observer_sees_exact_reverse_rollback_after_state_marker() {
    unica_workspace::actor::policy_contract_tests::retained_apply_observer_sees_exact_reverse_rollback_after_state_marker(policies());
}

#[test]
fn retained_apply_publication_preserves_exact_typed_causes_end_to_end() {
    unica_workspace::actor::policy_contract_tests::retained_apply_publication_preserves_exact_typed_causes_end_to_end(policies());
}

#[test]
fn prepared_apply_cache_authority_diagnostic_never_exposes_absolute_root() {
    unica_workspace::actor::policy_contract_tests::prepared_apply_cache_authority_diagnostic_never_exposes_absolute_root(policies());
}

#[test]
fn cache_preimage_races_fail_closed_and_preserve_foreign_names() {
    unica_workspace::actor::policy_contract_tests::cache_preimage_races_fail_closed_and_preserve_foreign_names(policies());
}

#[test]
fn cache_preimage_link_race_and_absent_chain_occupation_preserve_foreign_namespace() {
    if let Err(skip) = unica_workspace::actor::policy_contract_tests::cache_preimage_link_race_and_absent_chain_occupation_preserve_foreign_namespace(policies()) {
        unica_process::test_support::skip_named_host_capability(skip.capability, skip.reason);
    }
}

#[test]
fn malformed_state_fallback_restores_exact_bytes_when_later_gate_fails() {
    unica_workspace::actor::policy_contract_tests::malformed_state_fallback_restores_exact_bytes_when_later_gate_fails(policies());
}

#[test]
fn rollback_incomplete_failure_has_a_typed_non_success_category() {
    unica_workspace::actor::policy_contract_tests::rollback_incomplete_failure_has_a_typed_non_success_category(policies());
}

#[test]
fn apply_admission_and_dry_run_are_cache_tree_write_free() {
    unica_workspace::actor::policy_contract_tests::apply_admission_and_dry_run_are_cache_tree_write_free(policies());
}

#[test]
fn prepared_apply_dry_run_is_byte_identical_and_real_apply_commits_the_same_plan_once() {
    unica_workspace::actor::policy_contract_tests::prepared_apply_dry_run_is_byte_identical_and_real_apply_commits_the_same_plan_once(policies());
}

#[test]
fn apply_selection_rejects_v8project_kind_change_after_prepare() {
    unica_workspace::actor::policy_contract_tests::apply_selection_rejects_v8project_kind_change_after_prepare(policies());
}

#[test]
fn apply_selection_rejects_v8project_absence_to_appearance_after_prepare() {
    unica_workspace::actor::policy_contract_tests::apply_selection_rejects_v8project_absence_to_appearance_after_prepare(policies());
}

#[test]
fn apply_selection_rejects_autodetected_extension_membership_change() {
    unica_workspace::actor::policy_contract_tests::apply_selection_rejects_autodetected_extension_membership_change(policies());
}

#[test]
fn apply_selection_rejects_unselected_declared_parent_appearance() {
    unica_workspace::actor::policy_contract_tests::apply_selection_rejects_unselected_declared_parent_appearance(policies());
}

#[test]
fn apply_selection_rejects_unselected_non_platform_map_input_change() {
    unica_workspace::actor::policy_contract_tests::apply_selection_rejects_unselected_non_platform_map_input_change(policies());
}

#[test]
fn apply_selection_rejects_repaired_large_unselected_external_descriptor() {
    unica_workspace::actor::policy_contract_tests::apply_selection_rejects_repaired_large_unselected_external_descriptor(policies());
}

#[test]
fn apply_selection_dry_run_rejects_late_map_change_without_receipt() {
    unica_workspace::actor::policy_contract_tests::apply_selection_dry_run_rejects_late_map_change_without_receipt(policies());
}

#[test]
fn apply_selection_late_change_rolls_back_source_cache_revision_and_receipt() {
    unica_workspace::actor::policy_contract_tests::apply_selection_late_change_rolls_back_source_cache_revision_and_receipt(policies());
}

#[test]
fn apply_selection_rejects_autodetection_container_identity_replacement() {
    unica_workspace::actor::policy_contract_tests::apply_selection_rejects_autodetection_container_identity_replacement(policies());
}

#[test]
fn apply_preparation_preserves_all_six_typed_staging_error_kinds() {
    unica_workspace::actor::policy_contract_tests::apply_preparation_preserves_all_six_typed_staging_error_kinds(policies());
}

#[test]
fn apply_preparation_classifies_an_occupied_absent_final_file_by_cause() {
    unica_workspace::actor::policy_contract_tests::apply_preparation_classifies_an_occupied_absent_final_file_by_cause(policies());
}

#[test]
fn prepared_apply_cleanup_race_surfaces_a_relative_actor_diagnostic() {
    if let Err(skip) = unica_workspace::actor::policy_contract_tests::prepared_apply_cleanup_race_surfaces_a_relative_actor_diagnostic(policies()) {
        unica_process::test_support::skip_named_host_capability(skip.capability, skip.reason);
    }
}

#[test]
fn prepared_apply_rejects_stale_revision_source_change_cancellation_and_deadline_before_publication(
) {
    unica_workspace::actor::policy_contract_tests::prepared_apply_rejects_stale_revision_source_change_cancellation_and_deadline_before_publication(policies());
}

#[test]
fn prepared_apply_root_and_actor_capabilities_cannot_be_redirected_or_replayed() {
    unica_workspace::actor::policy_contract_tests::prepared_apply_root_and_actor_capabilities_cannot_be_redirected_or_replayed(policies());
}

#[test]
fn prepared_apply_root_replacement_during_commit_rolls_back_before_result() {
    unica_workspace::actor::policy_contract_tests::prepared_apply_root_replacement_during_commit_rolls_back_before_result(policies());
}

#[test]
fn postimage_validation_preserves_typed_root_containment_cause() {
    unica_workspace::actor::policy_contract_tests::postimage_validation_preserves_typed_root_containment_cause(policies());
}

#[test]
fn postimage_validation_preserves_typed_absent_chain_containment_cause() {
    unica_workspace::actor::policy_contract_tests::postimage_validation_preserves_typed_absent_chain_containment_cause(policies());
}

#[test]
fn prepared_apply_create_post_rename_failure_rolls_back_the_published_name() {
    unica_workspace::actor::policy_contract_tests::prepared_apply_create_post_rename_failure_rolls_back_the_published_name(policies());
}

#[test]
fn apply_admission_refuses_staged_state_from_another_actor_issued_authority() {
    unica_workspace::actor::policy_contract_tests::apply_admission_refuses_staged_state_from_another_actor_issued_authority(policies());
}

#[test]
fn prepared_apply_replace_post_rename_failure_restores_the_exact_preimage() {
    unica_workspace::actor::policy_contract_tests::prepared_apply_replace_post_rename_failure_restores_the_exact_preimage(policies());
}

#[test]
fn prepared_apply_replace_preserves_a_destination_replaced_at_the_mutation_boundary() {
    unica_workspace::actor::policy_contract_tests::prepared_apply_replace_preserves_a_destination_replaced_at_the_mutation_boundary(policies());
}

#[test]
fn prepared_apply_remove_preserves_a_destination_replaced_at_the_mutation_boundary() {
    unica_workspace::actor::policy_contract_tests::prepared_apply_remove_preserves_a_destination_replaced_at_the_mutation_boundary(policies());
}

#[test]
fn prepared_apply_dry_run_rejects_root_replacement_before_result() {
    unica_workspace::actor::policy_contract_tests::prepared_apply_dry_run_rejects_root_replacement_before_result(policies());
}

#[test]
fn saved_apply_plans_do_not_retain_per_change_descriptors() {
    unica_workspace::actor::policy_contract_tests::saved_apply_plans_do_not_retain_per_change_descriptors(policies());
}

#[test]
fn saved_source_diff_detects_changed_inputs_without_consuming_plan() {
    unica_workspace::actor::policy_contract_tests::saved_source_diff_detects_changed_inputs_without_consuming_plan(policies(), super::source_diff::layout);
}

#[test]
fn saved_apply_rebinds_nested_namespace_guards_to_the_execution_budget() {
    unica_workspace::actor::policy_contract_tests::saved_apply_rebinds_nested_namespace_guards_to_the_execution_budget(policies());
}

#[test]
fn saved_apply_large_retained_inputs_can_be_saved_and_executed() {
    unica_workspace::actor::policy_contract_tests::saved_apply_large_retained_inputs_can_be_saved_and_executed(policies());
}

#[test]
fn saved_apply_shared_cache_preimage_refuses_second_token_without_overwriting_either_source() {
    unica_workspace::actor::policy_contract_tests::saved_apply_shared_cache_preimage_refuses_second_token_without_overwriting_either_source(policies());
}

#[test]
fn saved_apply_read_only_input_conflict_explains_dependency_without_publishing() {
    unica_workspace::actor::policy_contract_tests::saved_apply_read_only_input_conflict_explains_dependency_without_publishing(policies());
}

#[test]
fn apply_policy_preserves_workspace_ancestor_precedence_over_source_local_policy() {
    unica_workspace::actor::policy_contract_tests::apply_policy_preserves_workspace_ancestor_precedence_over_source_local_policy(policies());
}

#[test]
fn apply_policy_absent_chain_rejects_nearer_policy_insertion_before_publication() {
    unica_workspace::actor::policy_contract_tests::apply_policy_absent_chain_rejects_nearer_policy_insertion_before_publication(policies());
}

#[test]
fn apply_policy_exact_file_rejects_byte_change_and_rename_replacement() {
    unica_workspace::actor::policy_contract_tests::apply_policy_exact_file_rejects_byte_change_and_rename_replacement(policies());
}

#[test]
fn apply_policy_stable_deny_evidence_allows_unrelated_dry_run_and_real_publication() {
    unica_workspace::actor::policy_contract_tests::apply_policy_stable_deny_evidence_allows_unrelated_dry_run_and_real_publication(policies());
}

#[test]
fn apply_policy_category_and_identity_transitions_are_rejected() {
    unica_workspace::actor::policy_contract_tests::apply_policy_category_and_identity_transitions_are_rejected(policies());
}

#[test]
fn apply_policy_dry_run_churn_is_write_free_and_returns_no_receipt() {
    unica_workspace::actor::policy_contract_tests::apply_policy_dry_run_churn_is_write_free_and_returns_no_receipt(policies());
}

#[test]
fn apply_policy_churn_before_source_publication_is_write_free() {
    unica_workspace::actor::policy_contract_tests::apply_policy_churn_before_source_publication_is_write_free(policies());
}

#[test]
fn apply_policy_churn_after_source_publication_rolls_back_all_retained_state() {
    unica_workspace::actor::policy_contract_tests::apply_policy_churn_after_source_publication_rolls_back_all_retained_state(policies());
}

#[test]
fn apply_policy_same_inode_churn_during_late_final_gate_rolls_back_all_retained_state() {
    unica_workspace::actor::policy_contract_tests::apply_policy_same_inode_churn_during_late_final_gate_rolls_back_all_retained_state(policies());
}

#[test]
fn apply_policy_same_ancestor_can_govern_two_worktrees_without_authority_aliasing() {
    unica_workspace::actor::policy_contract_tests::apply_policy_same_ancestor_can_govern_two_worktrees_without_authority_aliasing(policies());
}

#[test]
fn apply_policy_deadline_and_cancellation_during_capture_are_write_free() {
    unica_workspace::actor::policy_contract_tests::apply_policy_deadline_and_cancellation_during_capture_are_write_free(policies());
}

#[test]
fn apply_policy_capture_stops_after_first_retained_read_chunk_write_free() {
    unica_workspace::actor::policy_contract_tests::apply_policy_capture_stops_after_first_retained_read_chunk_write_free(policies());
}

#[test]
fn apply_policy_all_absent_capture_rejects_terminal_cancellation_and_deadline_write_free() {
    unica_workspace::actor::policy_contract_tests::apply_policy_all_absent_capture_rejects_terminal_cancellation_and_deadline_write_free(policies());
}

#[test]
fn apply_policy_deadline_and_cancellation_during_final_validation_roll_back() {
    unica_workspace::actor::policy_contract_tests::apply_policy_deadline_and_cancellation_during_final_validation_roll_back(policies());
}

#[test]
fn apply_policy_final_gate_stops_after_first_retained_read_chunk_and_rolls_back() {
    unica_workspace::actor::policy_contract_tests::apply_policy_final_gate_stops_after_first_retained_read_chunk_and_rolls_back(policies());
}

#[test]
fn apply_policy_warn_off_deny_database_and_malformed_match_v12() {
    unica_workspace::actor::policy_contract_tests::apply_policy_warn_off_deny_database_and_malformed_match_v12(policies());
}

#[test]
fn retained_apply_support_policy_evidence_does_not_add_a_third_writer_participant() {
    unica_workspace::actor::policy_contract_tests::retained_apply_support_policy_evidence_does_not_add_a_third_writer_participant(policies());
}

#[test]
fn prepared_apply_effects_are_retained_from_planner_to_result() {
    unica_workspace::actor::event_contract_tests::prepared_apply_effects_are_retained_from_planner_to_result(policies(), plan_events);
}

#[test]
fn prepared_apply_dry_run_returns_projected_effect_receipt_without_any_write() {
    unica_workspace::actor::event_contract_tests::prepared_apply_dry_run_returns_projected_effect_receipt_without_any_write(policies(), plan_events);
}

#[test]
fn prepared_apply_success_returns_committed_effect_receipt_after_one_commit() {
    unica_workspace::actor::event_contract_tests::prepared_apply_success_returns_committed_effect_receipt_after_one_commit(policies(), plan_events);
}

#[test]
fn event_implement_planner_integrates_with_actor_effect_publication_matrix() {
    unica_workspace::actor::event_contract_tests::event_implement_planner_integrates_with_actor_effect_publication_matrix(policies(), plan_events);
}

#[test]
fn event_implement_op_failure_returns_no_effect_receipt_and_preserves_all_state() {
    unica_workspace::actor::event_contract_tests::event_implement_op_failure_returns_no_effect_receipt_and_preserves_all_state(policies(), plan_events);
}

#[test]
fn retained_apply_effect_failure_matrix_rolls_back_and_returns_no_receipt() {
    unica_workspace::actor::event_contract_tests::retained_apply_effect_failure_matrix_rolls_back_and_returns_no_receipt(policies(), plan_events);
}

#[test]
fn retained_apply_effect_races_never_publish_or_return_effects() {
    unica_workspace::actor::event_contract_tests::retained_apply_effect_races_never_publish_or_return_effects(policies(), plan_events);
}

#[test]
fn real_effect_foreign_actor_replay_preserves_both_actor_states() {
    unica_workspace::actor::event_contract_tests::real_effect_foreign_actor_replay_preserves_both_actor_states(policies(), plan_events);
}

#[test]
fn real_effect_mutation_lane_cancellation_preserves_exact_state() {
    unica_workspace::actor::event_contract_tests::real_effect_mutation_lane_cancellation_preserves_exact_state(policies(), plan_events);
}

#[test]
fn real_effect_mutation_lane_deadline_preserves_exact_state() {
    unica_workspace::actor::event_contract_tests::real_effect_mutation_lane_deadline_preserves_exact_state(policies(), plan_events);
}

#[test]
fn real_effect_input_confirmation_cancellation_preserves_exact_state() {
    unica_workspace::actor::event_contract_tests::real_effect_input_confirmation_cancellation_preserves_exact_state(policies(), plan_events);
}

#[test]
fn real_effect_input_confirmation_deadline_preserves_exact_state() {
    unica_workspace::actor::event_contract_tests::real_effect_input_confirmation_deadline_preserves_exact_state(policies(), plan_events);
}

#[test]
fn real_effect_after_all_postimages_cancellation_rolls_back_exact_state() {
    unica_workspace::actor::event_contract_tests::real_effect_after_all_postimages_cancellation_rolls_back_exact_state(policies(), plan_events);
}

#[test]
fn real_effect_after_all_postimages_deadline_rolls_back_exact_state() {
    unica_workspace::actor::event_contract_tests::real_effect_after_all_postimages_deadline_rolls_back_exact_state(policies(), plan_events);
}

#[test]
fn saved_apply_survives_unchanged_read_and_rejects_same_byte_replacement() {
    unica_workspace::actor::event_contract_tests::saved_apply_survives_unchanged_read_and_rejects_same_byte_replacement(policies(), plan_events);
}

#[test]
fn saved_source_diff_reads_without_write_lane_and_survives_busy_or_cancelled_reads() {
    unica_workspace::actor::event_contract_tests::saved_source_diff_reads_without_write_lane_and_survives_busy_or_cancelled_reads(policies(), plan_events, super::source_diff::layout);
}

#[test]
fn saved_apply_executes_exact_preview_after_old_invocation_stops_and_replays_once() {
    unica_workspace::actor::event_contract_tests::saved_apply_executes_exact_preview_after_old_invocation_stops_and_replays_once(policies(), plan_events);
}

#[test]
fn saved_apply_stale_sources_refuse_without_replanning_and_replay_refusal() {
    unica_workspace::actor::event_contract_tests::saved_apply_stale_sources_refuse_without_replanning_and_replay_refusal(policies(), plan_events);
}

#[test]
fn saved_apply_rechecks_same_inode_same_length_hash_before_first_write() {
    unica_workspace::actor::event_contract_tests::saved_apply_rechecks_same_inode_same_length_hash_before_first_write(policies(), plan_events);
}

#[test]
fn saved_apply_refuses_a_disappeared_write_target_without_writing_cache() {
    unica_workspace::actor::event_contract_tests::saved_apply_refuses_a_disappeared_write_target_without_writing_cache(policies(), plan_events);
}

#[test]
fn saved_apply_wait_preserves_cancelled_and_deadline_errors_without_consuming_plan() {
    unica_workspace::actor::event_contract_tests::saved_apply_wait_preserves_cancelled_and_deadline_errors_without_consuming_plan(policies(), plan_events);
}

#[test]
fn saved_apply_execution_poison_is_internal_failure_and_writes_nothing() {
    unica_workspace::actor::event_contract_tests::saved_apply_execution_poison_is_internal_failure_and_writes_nothing(policies(), plan_events);
}

#[test]
fn saved_apply_plan_poison_is_registry_state_failure_and_writes_nothing() {
    unica_workspace::actor::event_contract_tests::saved_apply_plan_poison_is_registry_state_failure_and_writes_nothing(policies(), plan_events);
}

#[test]
fn saved_apply_plan_actor_mismatch_refuses_without_writes() {
    unica_workspace::actor::event_contract_tests::saved_apply_plan_actor_mismatch_refuses_without_writes(policies(), plan_events);
}

#[test]
fn saved_apply_entries_survive_later_plans_after_executor_releases_entry() {
    unica_workspace::actor::event_contract_tests::saved_apply_entries_survive_later_plans_after_executor_releases_entry(policies(), plan_events);
}

#[test]
fn saved_apply_token_is_actor_bound_and_foreign_execution_has_no_effect() {
    unica_workspace::actor::event_contract_tests::saved_apply_token_is_actor_bound_and_foreign_execution_has_no_effect(policies(), plan_events);
}

#[test]
fn apply_policy_foreign_actor_and_sibling_worktree_replay_are_rejected() {
    unica_workspace::actor::event_contract_tests::apply_policy_foreign_actor_and_sibling_worktree_replay_are_rejected(policies(), plan_events);
}

#[test]
fn remapped_names_and_profiles_do_not_share_index_or_coordination_state() {
    unica_workspace::actor::index_contract_tests::remapped_names_and_profiles_do_not_share_index_or_coordination_state(index_state_root);
}

#[test]
fn actor_bound_index_session_rejects_another_source_set_root() {
    unica_workspace::actor::index_contract_tests::actor_bound_index_session_rejects_another_source_set_root(index_active_build);
}

#[test]
fn external_cache_parent_components_publish_without_creating_cancelled_directories() {
    unica_workspace::actor::cache_authority_contract_tests::external_cache_parent_components_publish_without_creating_cancelled_directories(policies());
}

#[test]
fn external_cache_route_rejects_links_including_components_cancelled_by_parent() {
    if let Err(skip) = unica_workspace::actor::cache_authority_contract_tests::external_cache_route_rejects_links_including_components_cancelled_by_parent(policies()) {
        unica_process::test_support::skip_named_host_capability(skip.capability, skip.reason);
    }
}

#[test]
fn external_cache_ancestor_replacement_does_not_publish_source_or_foreign_cache() {
    unica_workspace::actor::cache_authority_contract_tests::external_cache_ancestor_replacement_does_not_publish_source_or_foreign_cache(policies());
}

#[test]
fn external_cache_missing_tail_occupation_keeps_foreign_entry_and_source() {
    unica_workspace::actor::cache_authority_contract_tests::external_cache_missing_tail_occupation_keeps_foreign_entry_and_source(policies());
}

#[test]
fn shared_workspace_alias_does_not_retarget_prepared_external_or_default_cache() {
    if let Err(skip) = unica_workspace::actor::cache_authority_contract_tests::shared_workspace_alias_does_not_retarget_prepared_external_or_default_cache(policies()) {
        unica_process::test_support::skip_named_host_capability(skip.capability, skip.reason);
    }
}

#[test]
fn workspace_suffix_alias_cannot_disguise_a_different_shared_cache_ancestor() {
    if let Err(skip) = unica_workspace::actor::cache_authority_contract_tests::workspace_suffix_alias_cannot_disguise_a_different_shared_cache_ancestor(policies()) {
        unica_process::test_support::skip_named_host_capability(skip.capability, skip.reason);
    }
}

#[test]
fn relative_cache_uses_process_cwd_when_workspace_has_an_alias() {
    if let Err(skip) = unica_workspace::actor::cache_authority_contract_tests::relative_cache_uses_process_cwd_when_workspace_has_an_alias(policies()) {
        unica_process::test_support::skip_named_host_capability(skip.capability, skip.reason);
    }
}

#[test]
fn cache_override_drive_prefix_follows_host_path_semantics_without_erasing_links() {
    if let Err(skip) = unica_workspace::actor::cache_authority_contract_tests::cache_override_drive_prefix_follows_host_path_semantics_without_erasing_links(policies()) {
        unica_process::test_support::skip_named_host_capability(skip.capability, skip.reason);
    }
}

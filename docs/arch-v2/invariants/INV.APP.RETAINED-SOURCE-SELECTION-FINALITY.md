---
id: INV.APP.RETAINED-SOURCE-SELECTION-FINALITY
status: active
governs: product
decision: DEC.2026-08-27.RETAINED-SOURCE-SELECTION-EVIDENCE-SLICE
check:
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::actor_admission_rejects_aggregate_exact_byte_budget
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::actor_admission_charges_repeated_exact_work_before_second_read
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::actor_admission_bounds_unique_retained_directories_without_ulimit
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::actor_admission_bounds_global_membership_across_external_source_sets
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::actor_admission_counts_repeated_membership_enumeration_globally
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::actor_admission_rejects_total_evidence_record_budget
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::actor_admission_rejects_route_and_name_byte_budget
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_selection_pass_checks_membership_budget_before_enumeration
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_selection_pass_checks_remaining_record_capacity_before_enumeration
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_selection_pass_checks_remaining_name_capacity_before_enumeration
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_selection_pass_rejects_before_unseen_member_child_open
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::membership_overflow_probe_never_retains_more_names_than_charged
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::membership_zero_work_rejects_before_enumeration
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::membership_child_record_cost_is_preflighted_before_open
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::membership_child_route_cost_is_preflighted_before_open
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_exact_read_never_appends_a_growth_chunk_past_the_limit
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_selection_pass_checks_record_budget_before_regular_open
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::actor_admission_comparison_honors_cancellation
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::actor_admission_comparison_honors_deadline
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_selection_pass_deduplicates_repeated_observations
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_selection_pass_rejects_inconsistent_regular_repeat
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_selection_pass_rejects_inconsistent_directory_repeat
  - crates/unica-coder/src/infrastructure/source_selection_evidence.rs::retained_selection_pass_rejects_inconsistent_membership_repeat
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_selection_rejects_v8project_kind_change_after_prepare
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_selection_rejects_v8project_absence_to_appearance_after_prepare
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_selection_rejects_autodetected_extension_membership_change
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_selection_rejects_unselected_declared_parent_appearance
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_selection_rejects_unselected_non_platform_map_input_change
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_selection_rejects_repaired_oversized_unselected_external_descriptor
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_selection_dry_run_rejects_late_map_change_without_receipt
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_selection_late_change_rolls_back_source_cache_revision_and_receipt
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_selection_rejects_autodetection_container_identity_replacement
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::prepared_apply_root_and_actor_capabilities_cannot_be_redirected_or_replayed
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_foreign_actor_and_sibling_worktree_replay_are_rejected
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_binding_rejects_a_same_path_directory_replacement
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::active_alias_reuses_actor_and_dropped_actor_recreates_a_new_instance
  - crates/unica-coder/src/infrastructure/daemon/server.rs::restart_request_does_not_claim_noncooperative_actor_released_in_process
  - crates/unica-coder/src/infrastructure/daemon/server.rs::working_task_recovery_is_resume_unsupported_without_apply_reexecution
  - crates/unica-coder/src/infrastructure/daemon/server.rs::view_find_admitted_snapshot_may_finish_after_map_change
  - crates/unica-coder/src/infrastructure/daemon/server.rs::semantically_equivalent_map_edit_reuses_actor_identity
  - crates/unica-coder/src/infrastructure/project_sources.rs::actor_admission_preserves_declared_external_processor_and_report_map
  - crates/unica-coder/src/infrastructure/project_sources.rs::actor_admission_external_config_dump_info_content_change_invalidates_evidence
  - crates/unica-coder/src/infrastructure/project_sources.rs::actor_admission_external_descriptor_absence_to_appearance_invalidates_evidence
  - crates/unica-coder/src/infrastructure/project_sources.rs::external_actor_positive_witness_uses_no_process_global_counter
scope: [app, source, platform, cache]
---

# Apply публикуется только при неизменном retained выборе источников

Actor и apply admission получают полную карту из двух совпавших retained
descriptor-relative проходов. Apply переносит невызываемое и несериализуемое
evidence всех входов карты до prepublication, dry-result и retained-final
границ. `v8project.yaml` и успешно содержательно классифицируемый
`ConfigDumpInfo.xml` удерживаются по exact bytes; oversized descriptor может
оставить recoverable `format_probe_error` только вместе с typed terminal
observation retained parent/name/identity и `length > maximum`; existence-only
marker удерживает retained parent,
имя и `FileIdentity`. Поэтому изменение exact content, absence, wrong-kind,
membership, terminal oversized class или physical identity любой строки
исключает result/receipt, а in-place bytes existence-only marker не образуют
ложную семантическую зависимость. Actor probe без terminal observation закрывает
admission.

Каждый проход дедуплицирует одинаковые наблюдения, отвергает противоречивые и
применяет общие, не per-source-set, retained-state пределы: 32 MiB canonical
exact bytes, 65 536 evidence records, 128 retained directories и 8 MiB
route/name bytes. Отдельные pass-global work пределы — 32 MiB суммы metadata
length каждого exact observation и 16 384 перечисленных членов. Перечисленные
члены включают повторы, отрицательные не XML и wrong-kind inputs. Exact repeat
списывает work до streaming compare с canonical bytes без второго полного
буфера. Membership учитывает применимые record/name/member capacity до
enumeration, directory capacity — до возможного child open, а exact reader —
следующий chunk до расширения. Regular handle закрывается после capture, первый
snapshot не держит handles, второй держит не более 128 directory capabilities.
Сравнение заимствованного evidence проверяет deadline/cancellation в ходе работы.
Поздний отказ восстанавливает существующими двумя writers source, cache и revision
state; отдельного writer или durable source-selection recipe нет.

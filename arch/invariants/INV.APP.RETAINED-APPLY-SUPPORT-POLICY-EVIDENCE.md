---
id: INV.APP.RETAINED-APPLY-SUPPORT-POLICY-EVIDENCE
status: active
governs: product
decision: DEC.2026-08-26.RETAINED-APPLY-SUPPORT-POLICY-EVIDENCE-SLICE
check:
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_preserves_workspace_ancestor_precedence_over_source_local_policy
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_absent_chain_rejects_nearer_policy_insertion_before_publication
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_exact_file_rejects_byte_change_and_rename_replacement
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_stable_deny_evidence_allows_unrelated_dry_run_and_real_publication
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_category_and_identity_transitions_are_rejected
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::retained_support_policy_candidate_parent_replacement_is_rejected
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::retained_support_policy_exact_and_oversized_reject_name_replacement_after_pre_read_identity
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::retained_support_policy_exact_rejects_name_replacement_after_retained_read_before_acceptance
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::retained_support_policy_exact_rejects_same_inode_change_between_stability_passes
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_dry_run_churn_is_write_free_and_returns_no_receipt
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_churn_before_source_publication_is_write_free
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_churn_after_source_publication_rolls_back_all_retained_state
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_same_inode_churn_during_late_final_gate_rolls_back_all_retained_state
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_foreign_actor_and_sibling_worktree_replay_are_rejected
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_same_ancestor_can_govern_two_worktrees_without_authority_aliasing
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_deadline_and_cancellation_during_capture_are_write_free
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_all_absent_capture_rejects_terminal_cancellation_and_deadline_write_free
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_deadline_and_cancellation_during_final_validation_roll_back
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_capture_stops_after_first_retained_read_chunk_write_free
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_final_gate_stops_after_first_retained_read_chunk_and_rolls_back
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_policy_warn_off_deny_database_and_malformed_match_v12
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::retained_support_policy_read_stops_before_post_read_when_pre_read_becomes_terminal
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::retained_support_policy_read_stops_after_first_chunk_when_terminal
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::retained_support_policy_second_pass_reuses_terminal_state_between_chunks
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::retained_support_policy_reader_preserves_limit_plus_one_in_64_kib_chunks
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::retained_support_policy_reader_retries_interrupted_after_partial_read
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::retained_support_policy_reader_stops_repeated_interrupts_at_terminal_state
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::retained_support_policy_reader_preserves_limit_plus_one_after_interrupt
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::terminal_pre_read_does_not_leave_after_read_hook_for_following_validation
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::support_policy_database_paths_distinguish_nested_sources_from_prefix_siblings
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::support_policy_candidate_search_stops_at_exact_twentieth_candidate
  - crates/unica-coder/src/infrastructure/support_policy_evidence.rs::support_policy_overlapping_chains_keep_first_occurrence_order_without_duplicates
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_support_policy_evidence_does_not_add_a_third_writer_participant
scope: [app, platform, source]
---

# Retained apply проверяет exact support-policy evidence до результата

Actor admission удерживает bounded V12 candidate order, включая fixed-name
отсутствия и policy выше worktree. Planner видит только `Deny`, `Warn` или
`Off`; `Warn` и `Off` выводятся только из retained regular-file identity с
exact bytes. Каждая evidence revalidation до publication, в конце dry run и в
late final gate с rollback выполняет два последовательных полных pass всей
ordered chain под одной absolute deadline/cancellation, повторяя admitted
category, physical identity и exact bytes. Каждый descriptor-relative retained
read соблюдает `limit + 1`, читает чанками не более 64 КиБ и проверяет ту же
deadline/cancellation до и после каждого чанка; отдельный блокирующий syscall не
обещается прерываемым. Actor mutation lane сериализует actor-owned writers. Это
bounded optimistic stabilization для обычной конечной внешней правки, а не
history-sensitive защита от arbitrary same-user/ABA writer, strict multi-object
linearizability или immutability до/через revision install. Evidence не
добавляет writer participant.

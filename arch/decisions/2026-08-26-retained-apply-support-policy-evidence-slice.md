---
id: DEC.2026-08-26.RETAINED-APPLY-SUPPORT-POLICY-EVIDENCE-SLICE
status: active
governs: product
realized:
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
supersedes: []
superseded-by: null
establishes: [INV.APP.RETAINED-APPLY-SUPPORT-POLICY-EVIDENCE]
---

# Retained apply удерживает support-policy как read-only actor evidence

**Решение.** Apply admission удерживает ordered bounded цепочку fixed-name
`.v8-project.json`, выведенную только из workspace actor и admitted source root.
Для предшествующих кандидатов удерживаются identity retained parent и
доказательство отсутствия, а для выбранного regular file — retained identity и
exact bytes. Wrong-kind, unreadable, malformed, unknown и oversized policy дают
fail-closed `Deny` без глобального отказа admission; их evidence удерживает
достаточную для неизменности `Deny` категорию. Только exact regular bytes могут
дать `Warn` или `Off`.

Каждая validation boundary под actor mutation lane выполняет два
последовательных полных descriptor-relative pass по всей retained ordered
candidate chain: до публикации, после dry-run revision confirmation и в
retained final gate после postimages. Оба pass используют одну исходную
absolute deadline/cancellation и повторно доказывают admitted category,
physical identity и exact authorizing bytes. Retained regular-file content
читается descriptor-relative с сохранением `limit + 1` semantics, чанками не
более 64 КиБ и с той же deadline/cancellation перед и после каждого чанка; это
cooperative bound между syscall, а не обещание прервать один зависший syscall.
Ошибка до публикации
write-free, поздняя ошибка использует существующий reverse rollback. Evidence
ничего не публикует и не становится третьим transaction participant: writers
остаются ровно `Source + WorkspaceCache`. Публичный wire-контракт не меняется.

**Почему.** Pure v0.13 planners не должны получать `WorkspaceContext`, path или
сырой policy, но обязаны сохранять V12 authorisation semantics. Два полных pass
дают bounded optimistic stabilization перед publication/result при
сериализации actor-owned writers и обычной конечной внешней правке. Это не
доказательство истории без изменений, strict multi-object linearizability,
защита от arbitrary same-user/ABA writer или неизменность policy до либо через
`ActiveRevisionReconciliation::install`.

**Цена.** Каждая validation boundary платит до 2x bounded validation I/O.
Policy больше 32 MiB не может авторизовать `Warn`/`Off` и трактуется как
`Deny`; source-map provenance и `SourceSetKind` остаются отдельным C0b/15D.

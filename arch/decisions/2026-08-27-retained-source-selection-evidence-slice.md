---
id: DEC.2026-08-27.RETAINED-SOURCE-SELECTION-EVIDENCE-SLICE
status: active
governs: product
realized:
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
supersedes: []
superseded-by: null
establishes: [INV.APP.RETAINED-SOURCE-SELECTION-FINALITY]
---

# Apply удерживает полное evidence выбора источников

**Решение.** Actor admission строит карту источников только через retained workspace capability и два полных descriptor-relative no-follow прохода. Admission принимается, когда полная каноническая семантика карты и физическое evidence обоих проходов совпали; сохраняется evidence второго прохода.

Evidence охватывает exact bytes `v8project.yaml` и каждого успешно содержательно классифицируемого `ConfigDumpInfo.xml`, named absence и wrong-kind от ближайшего существующего retained ancestor, identity всех пройденных каталогов, полный детерминированный membership использованных контейнеров и marker inputs каждой строки карты, включая неподдерживаемые и невыбранные строки. Маркер, семантика которого использует только существование, удерживает retained parent, имя и `FileIdentity`: replacement или wrong-kind отвергается, но in-place смена его неиспользуемых bytes не объявляется сменой карты. Оно не сериализуется, не клонируется, не хранится в durable task и не является actor key или digest. Recoverable oversized `ConfigDumpInfo.xml` оставляет typed terminal observation: retained parent, имя, `FileIdentity` и порог, выше которого находится metadata length. Только такое наблюдение сохраняет `format_probe_error`; прочая ошибка actor probe закрывает admission. Оба validation pass заново доказывают regular kind, ту же physical identity и класс `length > maximum`, поэтому in-place truncate/repair отвергается.

Один проход канонически дедуплицирует повторные наблюдения и отвергает противоречивые повторы. Его совокупные retained-state пределы: 32 MiB canonical exact bytes, 65 536 evidence records, 128 retained directory capabilities и 8 MiB route/name bytes. Отдельный pass-global work ledger допускает не более 32 MiB exact metadata length и 16 384 перечисленных членов по всем external source sets, включая повторы, не XML и wrong-kind. Каждое exact наблюдение списывает metadata length до content read; повтор сравнивается с canonical bytes bounded streaming без второго полного буфера. Это actor-authority envelope поверх parser/output limits в 1 024 source sets и 16 384 format-evidence rows, а не их умножение. Membership до enumeration учитывает применимые member, record и route/name capacity; исчерпанная directory capacity отвергает unseen member до child open, а exact reader проверяет длину следующего chunk до расширения буфера. Отказ возвращает стабильную provider error. Первый проход преобразуется в handle-free snapshot до второго; retained evidence второго держит не более 128 directory handles, а regular-file handle закрывается сразу после capture. Сравнение проходит по заимствованному каноническому evidence без клонирования bytes и проверяет deadline/cancellation до и внутри линейной по records/bytes работы.

Daemon создаёт actor из полного поддерживаемого Platform XML projection этого admission. Каждый apply admission заново получает resolved admission, сверяет projection и выбранный actor binding и перемещает sealed evidence через prepared batch. Два полных validation pass выполняются до публикации, после dry-run revision confirmation и в retained final gate после source/cache postimages. Поздний отказ использует существующий reverse rollback; writers и transaction participants остаются ровно `Source + WorkspaceCache`.

Уже admitted logical read завершает retained snapshot независимо от поздней смены карты; последующий invocation получает новую семантическую карту. Решение не заменяет V12 `ProjectSourceMapProvenance`, не меняет публичный wire-контракт и не обещает historical/ABA oracle между отдельными checkpoints. Focused actor discovery tests дополнительно подтверждают обычные external processor/report строки карты.

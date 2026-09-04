---
id: INV.APP.DAEMON-STORE-FAIL-STOP
status: active
governs: product
decision: DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE
check:
  - crates/unica-coder/src/application/invocation_store_actor.rs::blocked_store_call_is_bounded_without_releasing_worker_barrier
  - crates/unica-coder/src/application/invocation_store_actor.rs::queued_call_expires_behind_a_stuck_store_worker_without_late_execution
  - crates/unica-coder/src/application/invocation_store_actor.rs::failed_store_worker_returns_a_closed_unavailable_error
  - crates/unica-coder/src/infrastructure/task_store.rs::preallocated_create_collision_is_typed_and_never_replaces_the_first_record
  - crates/unica-coder/src/infrastructure/task_store.rs::uncertain_visible_create_counts_toward_capacity_before_and_after_reopen
  - crates/unica-coder/src/infrastructure/task_store.rs::pre_rename_create_failure_does_not_consume_retention_capacity
  - crates/unica-coder/src/infrastructure/task_store.rs::uncertain_visible_terminal_update_and_cancel_refresh_retention_state
  - crates/unica-coder/src/infrastructure/task_store.rs::held_file_writer_is_bounded_by_the_same_deadline_without_releasing_guard
  - crates/unica-coder/src/infrastructure/task_store.rs::active_and_nonexpired_terminal_records_are_never_evicted_at_capacity
  - crates/unica-coder/src/infrastructure/task_store.rs::expired_terminal_records_are_reclaimed_only_when_bounded_capacity_is_needed
  - crates/unica-coder/src/infrastructure/task_store.rs::create_does_not_rescan_a_directory_that_grew_beyond_the_recovery_bound
  - crates/unica-coder/src/infrastructure/task_store.rs::recovery_excess_is_typed_capacity_not_unbounded_enumeration_or_corruption
  - crates/unica-coder/src/infrastructure/task_store.rs::oversized_valid_record_is_rejected_before_unbounded_recovery_read
  - crates/unica-coder/src/infrastructure/task_store.rs::file_store_enforces_the_canonical_result_limit_before_publication
  - crates/unica-coder/src/infrastructure/task_store.rs::record_serialization_uses_the_original_store_deadline_without_reset
  - crates/unica-coder/src/infrastructure/daemon/server.rs::restart_request_does_not_claim_noncooperative_actor_released_in_process
  - crates/unica-coder/src/infrastructure/daemon/mod.rs::process_death_owns_fail_stop_handoff_and_recovery
scope: [app, cache]
---

# Daemon store bounded, а fail-stop завершается только смертью процесса

Executor обращается к sole-writer store только через один serial store actor с
bounded channel и общим absolute monotonic deadline/cancellation. Зависший
adapter или syscall не удерживает caller: daemon закрывает admission и просит
процесс завершиться, не выдавая staged result и не запуская domain execution
повторно.

File store ограничивает writer acquisition, размер record, recovery enumeration
и число retained records. Create использует preallocated TaskId и атомарную
публикацию без замены; collision типизирован. При capacity удаляются только
истёкшие terminal records; active и неистёкшие records не вытесняются.
Успешный rename изменяет in-memory retention catalog до fallible directory
sync, поэтому видимый uncertain record учитывается немедленно и после reopen.
Pre-rename failure catalog не меняет. Record ограничен 8 MiB canonical result
плюс 64 KiB envelope; serialization использует исходный store deadline.

`RestartRequested` не означает, что in-process resources уже освобождены.
Listener закрывается, PID-bound endpoint остаётся до смерти процесса, и только
после неё successor получает sole-writer ownership, заменяет stale endpoint и
закрывает оставшийся `Working` через recovery без второго execution.

---
id: INV.APP.DAEMON-STORE-FAIL-STOP
status: active
governs: product
decision: DEC.2026-09-07.DAEMON-V5-PRODUCTION-CUTOVER
check:
  - crates/unica-coder/src/application/receipt_ledger_actor.rs::reserve_panic_is_commit_uncertain_and_fail_stops_actor
  - crates/unica-coder/src/application/receipt_ledger_actor.rs::running_reserve_deadline_is_commit_uncertain_and_fail_stops_actor
  - crates/unica-coder/src/application/receipt_ledger_actor.rs::expired_command_queued_behind_running_reserve_never_reaches_the_port
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::commit_uncertain_is_returned_before_process_owned_fail_stop_retains_endpoint
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::every_fail_stop_store_error_is_written_without_reentering_the_actor
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::displaced_receipt_authority_fail_stops_until_process_death
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::capacity_never_lazily_expires_terminal_records_and_not_found_is_typed
  - crates/unica-coder/src/infrastructure/task_lifecycle_link_store_v5.rs::count_and_byte_entitlement_reject_second_reservation_before_task_store_create
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::noncooperative_prepare_forces_fail_stop_after_two_second_grace
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::task_store_capacity_after_reservation_is_invariant_violation_and_fail_stops
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

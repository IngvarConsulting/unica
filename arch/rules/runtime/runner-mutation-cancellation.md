---
id: INV.RUNTIME.RUNNER-MUTATION-CANCELLATION
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_cf_import.rs::upload_detaches_only_its_executing_runner_call
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::push_detaches_only_its_executing_runner_call
  - crates/unica-coder/src/infrastructure/daemon/v13_extensions.rs::only_mutating_extension_calls_detach_from_cancellation
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_create.rs::cancellation_after_create_keeps_the_confirmation_probe_and_receipt
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::restore_detaches_only_its_mutating_call
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::protected_mutation_preserves_success_and_failure_after_cancel_request
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::protected_mutation_does_not_arm_the_two_second_cancel_watchdog
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::inline_protected_mutation_cancel_does_not_arm_fail_stop_watchdog
  - crates/unica-coder/src/infrastructure/daemon/server.rs::mutating_runner_cancel_keeps_the_factual_receipt_over_the_v5_daemon_wire
  - crates/unica-coder/src/interfaces/mcp.rs::public_task_cancel_and_result_preserve_a_started_infobase_create_receipt
  - crates/unica-coder/src/interfaces/mcp.rs::public_native_cancel_waits_for_slow_job_attach_before_answering
  - crates/unica-coder/src/interfaces/mcp.rs::native_task_get_reports_late_cancel_request_without_claiming_cancellation
  - crates/unica-coder/src/infrastructure/platform/process.rs::protected_process_finishes_after_cancellation_on_every_host
  - crates/unica-coder/src/infrastructure/platform/process.rs::slow_job_attach_delays_cancel_without_starting_the_mutation_early
  - crates/unica-coder/src/domain/cancellation.rs::cancellation_waits_for_in_flight_protected_dispatch
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::completed_provider_receipt_survives_a_late_task_cancel_request
---

# Отмена не обрывает начатое изменение информационной базы

До запуска раннера отмена останавливает изменение. Предпросмотр и чтение
остаются отменяемыми. После запуска операции, изменяющей информационную базу,
Unica дожидается результата раннера и не завершает принудительно его дерево
из-за позднего запроса отмены. Подтверждение после создания базы относится к
тому же защищённому действию.

Если раннер подтвердил успех, поздняя отмена не заменяет его квитанцию на
`Cancelled`. В совместимых `unica.task.get/cancel` запрос отмены виден как
`cancelRequested: true`, а native Task передаёт его через `statusMessage`.
Фактический результат публикуется как завершённый. Политика не меняет
ограниченную очистку обычных управляемых процессов и операций чтения.

Если после запуска защищённый раннер не отвечает, задание остаётся `working`
до его фактического результата. Поздний запрос отмены не создаёт ложное
конечное состояние и не запускает принудительное завершение раннера. Если
демон перезапущен после отмеченного начала работы, но до надёжного сохранения
конечного результата, по [правилу восстановления заданий](task-recovery.md)
задание завершается `outcome_uncertain` без повторного исполнения.

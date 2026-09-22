---
id: INV.APP.RUNTIME-RESOURCE-TREE
check:
  - crates/unica-coder/src/infrastructure/platform/process.rs::runtime_sentinel_preserves_a_meaningful_inherited_fd198
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::system_runtime_job_keeps_resource_owned_after_leader_exit_until_descendant_dies
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::worker_supervises_initial_retained_ownership_until_proven_terminal
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::worker_supervises_fallback_retained_ownership_until_proven_terminal
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::worker_quarantines_poll_failure_until_later_terminal_proof
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::worker_quarantines_output_failure_until_later_eof_proof
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::stale_local_worker_retains_process_until_later_terminal_proof
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::stream_tail_read_failure_is_sticky_across_quarantine_probes
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::stream_tail_panic_is_sticky_across_quarantine_probes
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::worker_retains_initial_process_when_running_record_write_fails
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::worker_retains_fallback_process_when_running_record_write_fails
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::quarantine_thread_spawn_failure_retains_process_authority_and_active_lock
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::proven_childless_spawn_failure_releases_initial_active_lock
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::proven_childless_fallback_spawn_failure_releases_active_lock
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::waitid_authority_loss_makes_cancel_and_drop_send_zero_group_signals
  - crates/unica-coder/src/infrastructure/runtime_jobs.rs::reap_authority_loss_makes_cancel_and_drop_send_zero_group_signals
---

# Ресурс runtime освобождается после подтверждённого завершения

Завершение основного процесса не освобождает ресурс, пока жив принадлежащий
ему потомок: сохраняются состояние `Running` и `active.lock`.

Ошибка первичного или повторного запуска, опроса, чтения вывода либо записи
состояния не доказывает завершение. Сохранённый процесс остаётся под
наблюдением; даже ошибка запуска наблюдателя не отбрасывает право управления
процессом. Состояние `Lost` обозначает неопределённость и не разрешает запуск
замены. Ошибка или паника читателя вывода не исчезает при следующем опросе.

Освобождение требует подтверждённого завершения принадлежащего Unica дерева
и достижения конца обоих потоков вывода. Если уже при неудачном запуске
доказано отсутствие оставшихся процессов, блокировку можно снять сразу.
Запись результата и снятие блокировки соблюдают
[границу исходного каталога задания](retained-job-publication.md).

После потери подтверждённого права на Unix-группу процессов отмена и
освобождение объекта не посылают сигналы по её прежнему числовому идентификатору.

В Unix служебный дескриптор наблюдения за потомками не заменяет уже открытый
дескриптор, унаследованный запускаемым процессом.

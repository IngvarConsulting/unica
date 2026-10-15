---
id: INV.RLM.MAINTENANCE-STATUS
check:
  - crates/unica-coder/src/infrastructure/workspace_services.rs::real_index_queue_and_worker_state_reach_serialized_search_and_navigation
  - crates/unica-coder/src/infrastructure/workspace_services.rs::dropped_index_job_is_not_reported_as_running
  - crates/unica-coder/src/infrastructure/workspace_services.rs::maintenance_queue_execution_and_failure_keep_the_published_index_readable
  - crates/unica-coder/src/infrastructure/workspace_index.rs::worker_terminal_observation_tracks_real_failure_cancellation_and_publication
  - crates/unica-coder/src/infrastructure/workspace_index.rs::foreign_or_replaced_lock_never_proves_queued_or_running_maintenance
  - crates/unica-coder/src/infrastructure/workspace_index.rs::actor_bound_background_job_publishes_no_result_after_mid_command_root_swap
  - crates/unica-coder/src/infrastructure/rlm_navigation.rs::cancelled_or_unknown_maintenance_is_not_a_cancelled_read_or_a_wait_promise
---

# Ответ RLM различает очередь, выполнение и отказ запуска обновления

Ответ сообщает, выполняется ли обновление индекса, принято ли оно в очередь
или не удалось начать работу. Состояние самого индекса не доказывает,
что его обновление запущено.

Предлагать дождаться обновления можно при подтверждённой работе или принятом
задании. Уже существующего задания достаточно; новый запуск на каждый запрос
не требуется. Если состояние работы неизвестно, ответ явно сообщает об этом.
Постановка в очередь не означает начало выполнения, а начало — успешное
завершение.

Состояние обновления не запрещает
[чтение пригодного старого индекса](rlm-stale-results.md).

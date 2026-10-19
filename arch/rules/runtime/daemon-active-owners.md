---
id: INV.RUNTIME.DAEMON-ACTIVE-OWNERS
check:
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::owner_handoff_between_idle_reads_keeps_listener_and_exact_owner_alive
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::live_v5_owner_prevents_idle_listener_shutdown
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::daemon_idle_saved_apply_plan_preserves_instance_and_exact_replay_after_owner_release
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::daemon_idle_empty_runtime_still_exits_after_owner_release
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::daemon_idle_saved_apply_plan_does_not_prevent_explicit_stop
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::daemon_idle_running_index_producer_without_rpc_or_lease_preserves_instance_then_retires
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::daemon_idle_live_task_after_client_release_preserves_attempt_then_retires
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::daemon_idle_disconnected_inline_invocation_preserves_attempt_then_retires
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::daemon_idle_completed_index_lease_preserves_same_result_then_retires
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::warm_actor_expires_after_the_idle_ttl_and_is_rebuilt
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::idle_cleanup_retains_unknown_actor_state_without_rejecting_other_profiles
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::idle_listener_retains_unknown_registry_ownership
---

# Демон работает, пока у него остаются активные владельцы

Подключённый клиент, незавершённый вызов или задание, общая фоновая работа
и аренда, нужная выполняемой операции, удерживают демон. Сохранённый план
`apply` и его результат для точного повтора удерживают текущий экземпляр
после ухода клиента. То же относится к работающему index producer без RPC
и аренды и к результату общей index-работы, пока его удерживает потребитель.
Отключение клиента само по себе не отменяет оставшееся задание.

Владение непрерывно при передаче между допуском соединения, owner lease,
вызовом и публикуемым планом или общей работой. Решение о простое опирается
на согласованное наблюдение владельцев: переход не может дать ложное
отсутствие работы. Проверка читает состояние владельцев без обхода исходников
и снятия их ревизии. Ошибка чтения реестра не скрывается занятостью другой
стадии; неизвестное состояние не доказывает отсутствие владельцев.

После ухода всех владельцев демон выдерживает внутренний период простоя
и завершает работу; точная длительность не является публичным контрактом.
Пустой прогретый актор сам по себе демон не удерживает. Явная остановка
завершает демон и при сохранённом плане: сохранение токена после остановки
или перезапуска не обещается.

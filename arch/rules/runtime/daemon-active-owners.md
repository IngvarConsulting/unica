---
id: INV.RUNTIME.DAEMON-ACTIVE-OWNERS
check:
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::owner_handoff_between_idle_reads_keeps_listener_and_exact_owner_alive
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::live_v5_owner_prevents_idle_listener_shutdown
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::daemon_idle_saved_apply_plan_preserves_instance_and_exact_replay_after_owner_release
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::daemon_idle_empty_runtime_still_exits_after_owner_release
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::daemon_idle_saved_apply_plan_does_not_prevent_explicit_stop
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::daemon_idle_running_index_producer_without_rpc_or_lease_preserves_instance_then_retires
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::warm_actor_expires_after_the_idle_ttl_and_is_rebuilt
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::idle_cleanup_retains_unknown_actor_state_without_rejecting_other_profiles
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::idle_listener_retains_unknown_registry_ownership
gap: https://github.com/IngvarConsulting/unica/issues/980
---

# Демон работает, пока у него остаются активные владельцы

Подключённый клиент, незавершённый вызов или задание, общая фоновая работа
и аренда, нужная выполняемой операции, удерживают демон. Сохранённый план apply,
его результат для точного повтора и живая общая index-работа также удерживают
текущий экземпляр после ухода клиента. Отключение клиента
само по себе не отменяет оставшееся задание.

После ухода всех владельцев демон выдерживает внутренний период простоя
и завершает работу. Его точная длительность не является публичным контрактом.
TCP-проверки сохраняют экземпляр, исполняют план после ухода прежнего клиента
и получают тот же результат без второй записи. Работающий index producer
сохраняется без RPC и lease; после его завершения и ухода последнего потребителя
пустой демон освобождается. Само присутствие пустого warm-актора не удерживает
listener, неизвестное состояние реестра не считается пустым. Явная остановка
по-прежнему завершает демон; сохранение токена после неё или перезапуска
не обещается. Остальные разрывы жизненного цикла остаются в `gap`.

Переход от принятого handshake к owner lease не даёт ложного простоя:
listener сначала читает счётчик приёма, затем реестр владельцев. Lease
устанавливается до освобождения handshake, поэтому два последовательных
чтения не объединяют пустой старый реестр с нулём после перехода. Проверка
реестра выполняется и при занятом счётчике, сохраняя отказ при его повреждении.

Listener проверяет владение акторами после handshake, owner lease и активных
заданий. Новое обещание не теряется между этими наблюдениями: допущенный вызов
удерживает актор ещё до публикации плана или общей работы. Проверка читает
состояние владельцев, не обходит исходники и не снимает их ревизию.

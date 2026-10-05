---
id: INV.APP.DAEMON-ACTOR-CAPACITY
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::daemon_workspace_actor_admission_is_concurrent_and_fail_closed
  - crates/unica-coder/src/infrastructure/daemon/server.rs::default_actor_registry_admits_sixty_five_profiles_and_retains_exact_live_owners
  - crates/unica-coder/src/infrastructure/daemon/server.rs::saved_apply_token_survives_eight_other_workspace_profiles_and_replays_exactly
  - crates/unica-coder/src/infrastructure/daemon/server.rs::saved_apply_plan_survives_former_six_hundred_second_actor_idle_window
  - crates/unica-coder/src/infrastructure/daemon/server.rs::running_unowned_index_work_survives_last_call_and_actor_idle_window
  - crates/unica-coder/src/infrastructure/daemon/server.rs::completed_leased_index_work_survives_last_call_and_actor_idle_window
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::idle_cleanup_retains_unknown_actor_state_without_rejecting_other_profiles
  - crates/unica-coder/src/infrastructure/daemon/server.rs::saved_apply_token_survives_idle_cleanup_before_live_invocation_creates_plan
  - crates/unica-coder/src/infrastructure/daemon/server.rs::saved_apply_token_survives_warm_release_before_live_invocation_creates_plan
  - crates/unica-coder/src/infrastructure/daemon/server.rs::running_index_work_survives_idle_cleanup_before_live_invocation_creates_work
  - crates/unica-coder/src/infrastructure/daemon/server.rs::running_index_work_survives_warm_release_before_live_invocation_creates_work
---

# Допуск акторов сохраняет точную идентичность и владение выполняющейся работы

Демон не отказывает новому профилю по числу живых `WorkspaceActor`.
Параллельные обращения к одному профилю получают один актор. Повреждение
реестра даёт `workspace_registry_failed` и закрывает приём.
Актор, нужный допущенному или выполняющемуся вызову, не вытесняется:
исполнитель удерживает его до фактического завершения, в том числе после
запроса отмены. Мёртвые записи слабого реестра удаляются при следующем допуске.

После завершения вызова реестр удерживает актор, если в нём есть сохранённый
план, результат исполнения для replay либо живая запись общей index-работы.
Это включает работающего производителя без текущего вызова и завершённый
результат с удерживаемым lease. Число других профилей не вытесняет такой актор.
Неизвестное состояние из-за poison сохраняет владение; эта проверка очистки
сама по себе не отказывает другому профилю.

Интервал 600 секунд разрешает освобождение только актора без живого вызова
и обещанного состояния. Очистка сохраняет владельца ещё до появления плана
или общей работы: допущенный вызов может создать их позже.
При подмене именованного корня остаются проверки исходного корня
и создание нового экземпляра после освобождения прежнего; удержание не
разрешает исполнение старого плана в подменённом корне.

Граница этой гарантии — текущий демон, а не сохранение токенов после его
явной остановки или перезапуска. [Idle listener](../runtime/daemon-active-owners.md)
учитывает сохранённые планы, replay и общую index-работу перед завершением.

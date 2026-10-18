---
id: INV.WIRE.APPLY-REQUIRES-THE-FENCE
check:
  - crates/unica-coder/src/application/v13/apply.rs::planning_and_saved_plan_execution_have_disjoint_public_shapes
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::saved_apply_large_retained_inputs_can_be_saved_and_executed
  - crates/unica-coder/src/infrastructure/daemon/server.rs::saved_apply_more_than_256_plans_keep_earlier_tokens_without_writes
  - crates/unica-coder/src/infrastructure/daemon/server.rs::saved_apply_token_survives_eight_other_workspace_profiles_and_replays_exactly
  - crates/unica-coder/src/infrastructure/daemon/server.rs::saved_apply_token_survives_temporary_named_root_validation_failure
  - crates/unica-coder/src/infrastructure/daemon/server.rs::saved_apply_plan_survives_former_three_hundred_second_expiry
  - crates/unica-coder/src/infrastructure/daemon/server.rs::saved_apply_plan_survives_former_six_hundred_second_actor_idle_window
  - crates/unica-coder/src/infrastructure/daemon/server.rs::saved_apply_refusal_replays_after_former_actor_idle_window_without_replanning
---

# Применение исполняет сохранённый план и повторяет его результат

`unica.apply` с `at` и `ops` проверяет операции и возвращает план без записи.
Успешный план содержит `data.executionToken`. Для исполнения вызывающий
передаёт только `executionToken`: исполнитель использует сохранённую
транзакцию, не строя её повторно. Публичная схема разделяет эти два вида
вызова; смешанные аргументы, `dryRun` и `ifRev` не допускаются.

План сохраняет операции и аргументы, выбранные корни, прочитанные входы,
ожидаемые байты и эффекты. Токен указывает на этот план, а не на ревизию
конфигурации. Для другого адреса или операций нужен новый план.

Сохранение плана не отказывает по общей квоте байтов или числу сохранённых
планов. Допустимость нового ограничения задаёт
[правило обоснования ресурсов](../runtime/resource-limit-evidence.md).
Возраст токена и обращения к другим профилям не удаляют план в текущем демоне.
Сохранение следующего плана не освобождает предыдущие. Исполненный план
сохраняет точный результат, включая отказ, для повторного исполнения токена
без повторной публикации. Это не обещает сохранение токена после перезапуска
демона и не разрешает применение в подменённом корне.

Исполнение соблюдает [равенство входов плана](sources/projected-revision-equality.md),
отмену, сроки, проверки карты исходников и поддержки, а также
[гарантию отката](retained-apply-rollback.md). `view` не выдаёт токен для применения.

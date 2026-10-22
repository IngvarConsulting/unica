---
id: INV.APP.EXECUTION-AUTHORITY
check:
  - crates/unica-execution/src/coordinator/tests.rs::accepted_and_begin_commits_are_required_before_a_single_run_permit
  - crates/unica-execution/src/coordinator/tests.rs::exact_readback_and_duplicate_callback_never_repeat_the_run_permit
  - crates/unica-execution/src/coordinator/tests.rs::metadata_conflict_redecides_factual_outcome_without_new_execution
  - crates/unica-execution/src/coordinator/acceptance_tests.rs::acceptance_pending_ids_are_unique_across_groups_before_any_commit
  - crates/unica-execution/src/coordinator/acceptance_tests.rs::acceptance_submit_cancel_begin_permutations_never_replay
  - crates/unica-execution/src/coordinator/acceptance_tests.rs::acceptance_cancel_outcome_expose_permutations_preserve_factual_mutation
  - crates/unica-execution/src/coordinator/acceptance_tests.rs::acceptance_unknown_begin_never_grants_run_and_recovery_never_replays
  - crates/unica-execution/src/coordinator/acceptance_tests.rs::acceptance_last_cancel_and_attach_are_ordered_by_confirmed_membership
  - crates/unica-execution/src/coordinator/acceptance_tests.rs::acceptance_retired_membership_cannot_cancel_its_successor
  - crates/unica-execution/src/coordinator/acceptance_tests.rs::acceptance_writable_capabilities_cannot_be_cloned
  - crates/unica-execution/tests/platform/store/owner_tests.rs::same_epoch_foreign_commit_is_rejected_before_io_without_poisoning_owner
  - crates/unica-execution/tests/platform/store/file_tests.rs::physical_cas_readback_redecides_the_same_factual_outcome
  - crates/unica-execution/tests/platform/store/process_tests.rs::forced_process_death_recovers_accepted_begun_and_durable_outcome
gap: https://github.com/IngvarConsulting/unica/issues/1743
---

# Переходы принятой попытки имеют одного владельца

Принятие, разрешение начала, намерение отмены, конечный результат и права его
выдачи принадлежат одной модели исполнения. Direct и Task наблюдают ту же
попытку и не передают её другому владельцу. Идентичность попытки отличается
от версии её изменяемых сведений; оба идентификатора вызова уникальны во всей
области хранения, включая раннюю отмену и неподтверждённое принятие.

Решение модели не является сохранённым фактом. Только точное подтверждение
соответствующего перехода разрешает внешнее действие. Подтверждённое начало
выдаёт одноразовое полномочие запуска; повтор подтверждения и восстановление
не выдают его снова. Чужое или запоздалое подтверждение не завершает другой
переход. Неопределённость authoritative commit закрывает приём по
[правилу остановки](receipt-store-fail-stop.md).

Исполнитель возвращает фактический результат разрешённой попытки, а не пишет
её состояние. Отмена, наблюдение и конфликт версии сохранения не отбрасывают
этот результат и не разрешают повтор исполнения. Восстановление соблюдает
[правило незавершённых задач](task-recovery.md).

Общее чтение согласует состав держателей и отмену последнего с жизненным
циклом producer. Временные aliases не становятся долговечными. Полномочие
старого состава держателей не действует на преемника; позднее присоединение
не возвращает отменённого producer к исполнению.

Целевая гарантия принята в #1743. Чистая модель, её подключение к рабочим
маршрутам и атомарность выбранного хранилища проверяются раздельно; до
завершения перехода старые writers остаются известным разрывом.

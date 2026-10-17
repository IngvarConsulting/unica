---
id: INV.APP.TASK-RETIREMENT
check:
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::task_bind_direct_ack_and_receipt_terminal_expiry_release_exact_quota
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::terminal_retirement_is_exact_explicit_and_reconciles_absence_after_uncertain_delete
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::retirement_begin_failure_stays_with_its_record
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::retirement_authorize_failure_stays_with_its_record
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::retirement_delete_failure_stays_with_its_record
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::retirement_finalize_failure_stays_with_its_record
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::uncertain_retirement_delete_stays_with_its_record_and_reconciles_on_retry
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::retirement_failure_after_uncertain_link_commit_serves_the_durable_catalog
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::retirement_retry_pause_doubles_up_to_an_hour_without_an_attempt_limit
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::reopened_retirement_rejects_old_authority_and_preserves_indexes_until_confirmed_delete
  - crates/unica-coder/src/infrastructure/task_lifecycle_link_store_v5.rs::finalized_retirement_removes_exact_link_and_both_indexes_across_reopen
---

# Очистка задания согласуется с его квитанцией

Очистке подлежит только завершённое задание после истечения срока,
отсчитанного от конечного результата. Перед удалением сохраняется намерение
очистить именно эту запись. После сбоя очистка продолжает это намерение
и различает удалённую запись, подтверждённое отсутствие, неопределённый
исход удаления и несовпадение идентичности.

Исчезновение активной задачи при оставшейся связи с квитанцией не считается
успешной очисткой: демон закрывает приём и требует перезапуска.
Сохранённое намерение очистки при уже удалённом задании — незавершённая
очистка: чтение такого задания не считает это порчей и приём не закрывает.

Очистка не является условием ответа. Если очистить одну запись не удалось
на любом шаге, запись сохраняет подтверждённое состояние, причина пишется
в stderr демона, а остальные просроченные записи очищаются. `task.get`,
`task.result` и `task.cancel` по другим заданиям, в том числе из другой
рабочей копии, отвечают как обычно. Следующая попытка для этой записи —
не раньше чем через минуту; пауза удваивается до часа без ограничения
числа попыток.

После отказа хранилища дальнейшая очистка согласуется с сохранённым
каталогом связей. Подтверждённое изменение связи, в том числе другого
задания, не теряется из-за устаревшего состояния в памяти. Повтор может
продолжить очистку с предыдущего допустимого этапа, подтверждённого каталогом.
Если каталог не читается или корень хранилища заменён, отказ относится
к хранилищу целиком, и демон закрывает приём по
[правилу остановки](receipt-store-fail-stop.md).

После подтверждённого удаления задания удаляются его связь и оба индекса
квитанции. Пока исход удаления не подтверждён, они сохраняются.

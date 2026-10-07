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
gap: https://github.com/IngvarConsulting/unica/issues/945
---

# Очистка задания согласуется с его квитанцией

Очистке подлежит только завершённое задание после истечения срока,
отсчитанного от конечного результата. Перед удалением сохраняется
намерение очистить именно эту запись. После сбоя очистка продолжает это
намерение и различает удалённую запись, подтверждённое отсутствие,
неопределённый исход удаления и несовпадение идентичности.

Исчезновение активной задачи при оставшейся связи с квитанцией не считается
успешной очисткой: демон закрывает приём и требует перезапуска.
Сохранённое намерение очистки при уже удалённом задании — незавершённая
очистка: чтение такого задания не считает это порчей и приём не закрывает.

Очистка не является условием ответа. Если очистить одну запись не удалось
на любом шаге, запись сохраняет подтверждённое состояние, причина пишется
в stderr демона, а остальные просроченные записи очищаются. `task.get`,
`task.result` и `task.cancel` по другим заданиям, в том числе из другой
рабочей копии, отвечают как обычно. Следующая попытка для этой записи —
не раньше чем через минуту; пауза удваивается до часа и не ограничивает
число попыток. Так, в #1035 связь одной записи не проходила предел размера
и опрос любого задания отвечал отказом; теперь такая запись ждёт повтора,
а опрос остальных заданий возвращает их результат.

После отказа хранилища на шаге очистки демон перечитывает сохранённый
каталог связей и дальше работает от него, а не от копии в памяти. Изменение
связи другого задания, записанное до перечитывания, может вернуть запись
к предыдущему допустимому состоянию её очистки; повтор продолжает очистку
с него. Если каталог не читается или корень хранилища заменён, отказ
относится к хранилищу целиком, и демон закрывает приём по
[правилу остановки](receipt-store-fail-stop.md).

После подтверждённого удаления задания удаляются его связь и оба индекса
квитанции. Пока исход удаления не подтверждён, они сохраняются. Проверка
реальных индексов ещё не завершена: прежний отчёт восстанавливал их
из списка связей и не доказывал состояние самих индексов.

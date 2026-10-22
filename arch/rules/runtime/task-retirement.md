---
id: INV.APP.TASK-RETIREMENT
check:
  - crates/unica-coder/tests/platform/receipt_runtime.rs::retirement_begin_failure_stays_with_its_record
  - crates/unica-coder/tests/platform/receipt_runtime.rs::retirement_authorize_failure_stays_with_its_record
  - crates/unica-coder/tests/platform/receipt_runtime.rs::retirement_delete_failure_stays_with_its_record
  - crates/unica-coder/tests/platform/receipt_runtime.rs::retirement_finalize_failure_stays_with_its_record
  - crates/unica-coder/tests/platform/receipt_runtime.rs::uncertain_retirement_delete_stays_with_its_record_and_reconciles_on_retry
  - crates/unica-coder/tests/platform/receipt_runtime.rs::retirement_failure_after_uncertain_link_commit_serves_the_durable_catalog
  - crates/unica-coder/tests/platform/receipt_runtime.rs::retirement_retry_pause_doubles_up_to_an_hour_without_an_attempt_limit
  - crates/unica-coder/tests/platform/receipt_runtime.rs::reopened_retirement_rejects_old_authority_and_preserves_indexes_until_confirmed_delete
gap: https://github.com/IngvarConsulting/unica/issues/1743
---

# Очистка задания согласуется с его квитанцией

Очистке подлежит только завершённое задание, для которого владелец обещаний
результата подтвердил освобождение всех удерживающих прав. Возраст записи,
отсутствие наблюдателя или истечение срока ожидания такого разрешения
не заменяют. До отдельно принятой политики освобождения terminal Task
и его результат не удаляются по возрасту.

Перед удалением сохраняется намерение очистить именно эту запись. После сбоя очистка продолжает это намерение
и различает удалённую запись, подтверждённое отсутствие, неопределённый
исход удаления и несовпадение идентичности.

Исчезновение активной задачи при оставшейся связи с квитанцией не считается
успешной очисткой: демон закрывает приём и требует перезапуска.
Сохранённое намерение очистки при уже удалённом задании — незавершённая
очистка: чтение такого задания не считает это порчей и приём не закрывает.

Очистка не является условием ответа. Если очистить одну запись не удалось
на любом шаге, запись сохраняет подтверждённое состояние, причина пишется
в stderr демона, а остальные разрешённые к удалению записи очищаются. `task.get`,
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

Пока исход удаления задания не подтверждён, его связь и оба индекса
квитанции сохраняются. После подтверждения можно убрать лишь те данные,
которые больше не нужны для обещанного наблюдения;
[след точной идентичности вызова](invocation-at-most-once.md) сохраняется.

Проверки подтверждают точность удаления, отказ устаревшего права,
восстановление намерения и локальность ошибок на прежнем пути v5.
Возрастное основание удаления в реализации ещё действует; новое разрешение
владельца обещаний и сохранение dedup trace после очистки остаются в `gap`.

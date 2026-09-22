---
id: INV.APP.DAEMON-STORE-FAIL-STOP
check:
  - crates/unica-coder/src/application/receipt_ledger_actor.rs::expired_direct_terminal_queued_behind_running_reserve_never_reaches_the_port
  - crates/unica-coder/src/application/receipt_ledger_actor.rs::reserve_panic_is_commit_uncertain_and_fail_stops_actor
  - crates/unica-coder/src/application/receipt_ledger_actor.rs::running_reserve_deadline_is_commit_uncertain_and_fail_stops_actor
  - crates/unica-coder/src/application/receipt_ledger_actor.rs::expired_command_queued_behind_running_reserve_never_reaches_the_port
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::commit_uncertain_is_returned_before_process_owned_fail_stop_retains_endpoint
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::every_fail_stop_store_error_is_written_without_reentering_the_actor
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::displaced_receipt_authority_fail_stops_until_process_death
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::noncooperative_prepare_forces_fail_stop_after_two_second_grace
gap: https://github.com/IngvarConsulting/unica/issues/984
---

# Неопределённая запись о вызове останавливает приём работы

Если срок команды истёк в очереди до обращения к хранилищу, команда
не выполняется и возвращает обычную ошибку срока. Хранилище остаётся доступным.

Если запись уже началась и завершение нельзя подтвердить из-за паники
или истечения срока, демон сообщает о неопределённом результате записи.
Новые команды к этому владельцу хранилища не допускаются. Подготовленный
результат не выдаётся как подтверждённый, предметная работа не повторяется.

Демон прекращает принимать работу и запрашивает завершение процесса.
Запрос перезапуска сам по себе не освобождает права на хранилище: запись
подключения остаётся связанной с живым PID, а другой владелец не занимает
его место до смерти процесса. Подмена каталога хранилища также закрывает приём.

Подготовка конечного ответа, ожидание в очереди и сохранение расходуют
один срок публикации. Начало следующего этапа не даёт нового бюджета.
Проверка всего пути, включая подготовку ответа до постановки в очередь,
остаётся в `gap`.

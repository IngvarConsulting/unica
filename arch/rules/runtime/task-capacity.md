---
id: INV.APP.DAEMON-TASK-CAPACITY
check:
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::capacity_never_lazily_expires_terminal_records_and_not_found_is_typed
  - crates/unica-coder/src/infrastructure/task_lifecycle_link_store_v5.rs::count_and_byte_entitlement_reject_second_reservation_before_task_store_create
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::task_store_capacity_after_reservation_is_invariant_violation_and_fail_stops
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::task_bind_direct_ack_and_receipt_terminal_expiry_release_exact_quota
  - crates/unica-coder/src/infrastructure/task_lifecycle_link_store_v5.rs::reservation_consumes_task_store_slot_before_materialization_and_reopens_exactly
  - crates/unica-coder/src/infrastructure/task_lifecycle_link_store_v5.rs::task_bound_terminal_and_retirement_transitions_are_exact_cas_and_reopen_stable
gap: https://github.com/IngvarConsulting/unica/issues/948
---

# Место для фонового задания резервируется до его создания

До создания задания резервируются место в `TaskStore` и предельный размер
его связи с квитанцией. Нехватка любого из этих ресурсов отклоняет
резервирование. Учитываются и созданные задания, и ещё не использованные
резервации; предел хранилища — 4096 записей. Связь с квитанцией занимает не более
1 КиБ, общий пул связей — не более 4 МиБ.

Хранилище не удаляет завершённое задание при чтении или нехватке места,
даже если срок хранения прошёл. Удалением управляет общий жизненный цикл
задачи и её квитанции.

Если место уже зарезервировано, а `TaskStore` всё же сообщает о нехватке,
это нарушение внутренней гарантии. Демон сохраняет намерение передачи
и резервацию, закрывает приём и завершает процесс. Он не освобождает место
ценой потери подготовленного результата и не запускает работу повторно.

Если места для связи с TaskStore не хватает до начала работы, квитанция
завершается с `task_capacity`. После начала результат остаётся в квитанции;
после сбоя без сохранённого результата возвращается `outcome_uncertain`.
Уже сохранённый результат и принятая отмена не заменяются ошибкой нехватки
места. Новая запись TaskStore не создаётся, другие задания не вытесняются.
Эта реакция ещё не подключена к обычному пути демона: прежние сценарии
самостоятельно вызывали её внутренние методы. Требуются подключение
и проверка настоящего вызова, в том числе с принятой отменой.

После передачи в TaskStore связь занимает одну ограниченную запись.
Смена стадии не сохраняет вторую активную квитанцию того же вызова.

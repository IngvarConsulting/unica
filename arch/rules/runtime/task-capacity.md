---
id: INV.APP.DAEMON-TASK-CAPACITY
check:
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::capacity_never_lazily_expires_terminal_records_and_not_found_is_typed
  - crates/unica-coder/src/infrastructure/task_lifecycle_link_store_v5.rs::count_and_byte_entitlement_reject_second_reservation_before_task_store_create
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::task_store_capacity_after_reservation_is_invariant_violation_and_fail_stops
---

# Место для фонового задания резервируется до его создания

До создания задания резервируются место в `TaskStore` и предельный размер
его связи с квитанцией. Нехватка любого из этих ресурсов отклоняет
резервирование. Учитываются и созданные задания, и ещё не использованные
резервации; предел хранилища — 4096 записей.

Хранилище не удаляет завершённое задание при чтении или нехватке места,
даже если срок хранения прошёл. Удалением управляет общий жизненный цикл
задачи и её квитанции.

Если место уже зарезервировано, а `TaskStore` всё же сообщает о нехватке,
это нарушение внутренней гарантии. Демон сохраняет намерение передачи
и резервацию, закрывает приём и завершает процесс. Он не освобождает место
ценой потери подготовленного результата и не запускает работу повторно.

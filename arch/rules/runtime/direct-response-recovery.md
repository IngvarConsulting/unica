---
id: INV.APP.DIRECT-RESPONSE-RECOVERY
check:
  - crates/unica-coder/src/interfaces/daemon_router.rs::lost_submit_response_is_recovered_by_the_exact_key_without_a_second_submission
  - crates/unica-coder/src/interfaces/daemon_router.rs::malformed_direct_receipt_is_recovered_by_key_and_only_the_strict_one_is_acknowledged
  - crates/unica-coder/src/interfaces/daemon_router.rs::direct_terminal_is_projected_before_it_is_acknowledged_with_the_exact_digest
  - crates/unica-coder/src/interfaces/daemon_router.rs::failed_and_cancelled_direct_terminals_answer_closed_errors_after_acknowledgement
---

# Потерянный прямой ответ восстанавливается без повторной отправки операции

При потере ответа или несовпадении хеша результата frontend запрашивает
квитанцию по исходному ключу. Повторно отправлять предметную операцию нельзя.
Квитанция с неверным хешем не подтверждается.

Для проверенного результата frontend формирует ответ клиенту и отправляет
подтверждение с точными ключом и хешем. Ошибка подтверждения не отнимает уже
полученный результат. Завершение с ошибкой или отменой также подтверждается
и возвращает закрытую причину, без произвольного текста демона.

Подтверждение означает, что frontend подготовил окончательный ответ.
Оно не доказывает, что клиент получил или обработал этот ответ.

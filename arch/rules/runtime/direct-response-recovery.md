---
id: INV.APP.DIRECT-RESPONSE-RECOVERY
check:
  - crates/unica-coder/src/interfaces/daemon_router.rs::pending_receipt_is_polled_until_its_original_budget_settles
  - crates/unica-coder/src/interfaces/daemon_router.rs::pending_receipt_beyond_the_frontend_cutoff_is_the_reserved_task_not_a_retry
  - crates/unica-coder/src/interfaces/daemon_router.rs::lost_submit_response_is_recovered_by_the_exact_key_without_a_second_submission
  - crates/unica-coder/src/interfaces/daemon_router.rs::lost_submit_response_follows_replaced_endpoint_without_resubmitting
  - crates/unica-coder/src/interfaces/daemon_router.rs::malformed_direct_receipt_is_recovered_by_key_and_only_the_strict_one_is_acknowledged
  - crates/unica-coder/src/interfaces/daemon_router.rs::direct_terminal_is_projected_before_it_is_acknowledged_with_the_exact_digest
  - crates/unica-coder/src/interfaces/daemon_router.rs::failed_and_cancelled_direct_terminals_answer_closed_errors_after_acknowledgement
  - crates/unica-coder/src/interfaces/daemon_router.rs::submit_answer_at_the_frame_limit_is_read_and_one_byte_over_or_cut_is_recovered
gap: https://github.com/IngvarConsulting/unica/issues/1743
---

# Потерянный прямой ответ восстанавливается без повторной отправки операции

При потере ответа или несовпадении хеша результата frontend запрашивает
квитанцию по исходному ключу. Повторно отправлять предметную операцию нельзя.
Квитанция с неверным хешем не подтверждается.

Для проверенного результата frontend формирует ответ клиенту и отправляет
подтверждение с точными ключом, хешем и capability владельца. После выдачи
Task поздний Direct не разрешает ACK; этот необратимый выбор описан в
[правиле выдачи](execution-delivery.md). Ошибка подтверждения не отнимает уже
полученный результат. Завершение с ошибкой или отменой также подтверждается
и возвращает закрытую причину, без произвольного текста демона.

Подтверждение означает, что frontend подготовил окончательный ответ.
Оно не доказывает, что клиент получил или обработал этот ответ.

Если восстановление встречает ещё исполняющуюся квитанцию, frontend
ждёт окончания её первоначального бюджета в пределах собственного срока.
Истечение срока не вызывает повторной отправки предметной операции.

Существующие проверки подтверждают восстановление и сохранение готового ответа
при отказе ACK. Новая capability и необратимый выбор Direct/Task ещё требуют
проверки продуктивного frontend; это граница `gap`.

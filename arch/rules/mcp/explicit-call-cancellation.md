---
id: INV.WIRE.COOPERATIVE-CANCELLATION
check:
  - crates/unica-coder/src/interfaces/mcp.rs::canonical_manual_cancellation_before_submit_reserves_exact_cancel_before_admission
  - crates/unica-coder/src/interfaces/mcp.rs::canonical_manual_cancellation_before_task_id_stops_only_target_and_preserves_ping
  - crates/unica-coder/src/interfaces/mcp.rs::canonical_manual_cancellation_eof_before_handler_preserves_registered_intent
  - crates/unica-coder/src/interfaces/mcp.rs::canonical_manual_cancellation_lost_cancel_answer_reconciles_original_key_without_replay
  - crates/unica-coder/src/interfaces/mcp.rs::canonical_manual_cancellation_after_partial_json_finishes_the_same_frame
  - crates/unica-coder/src/interfaces/mcp.rs::canonical_manual_cancellation_shared_index_stops_only_its_consumer
  - crates/unica-coder/src/interfaces/mcp.rs::canonical_manual_cancellation_protected_create_preserves_external_receipt_without_replay
  - crates/unica-coder/src/interfaces/mcp.rs::canonical_manual_cancellation_after_completed_before_flush_preserves_exact_terminal
  - crates/unica-coder/src/interfaces/mcp.rs::canonical_manual_cancellation_compatibility_wait_releases_only_observation
  - crates/unica-coder/src/interfaces/mcp.rs::canonical_manual_cancellation_reused_id_keeps_old_control_and_targets_new_receipt
  - crates/unica-coder/src/interfaces/mcp.rs::canonical_manual_cancellation_native_reuse_does_not_inherit_old_suppression
  - crates/unica-coder/src/interfaces/mcp.rs::canonical_manual_cancellation_sole_engine_waiter_preserves_delivery_and_pinned_artifact
  - crates/unica-coder/src/interfaces/mcp.rs::canonical_manual_cancellation_normal_response_does_not_cancel_daemon_operation
  - crates/unica-coder/src/interfaces/mcp.rs::canonical_manual_cancellation_eof_preserves_accepted_work_and_exact_recovery_without_replay
  - crates/unica-coder/src/interfaces/mcp.rs::canonical_manual_cancellation_stdio_drain_does_not_cancel_held_daemon_admission
  - crates/unica-coder/src/interfaces/mcp.rs::canonical_manual_cancellation_eof_waits_for_accepted_control_confirmation
  - crates/unica-coder/src/interfaces/mcp.rs::canonical_manual_cancellation_sdk_refusal_after_eof_retires_registered_request
---

# Явная отмена вызова доходит до исполнителя

Если клиент явно отменяет выполняющийся MCP-вызов, Unica передаёт отмену
его исполнителю, даже когда номер фонового задания ещё не выдан.
Исполнитель прекращает работу в безопасной точке; сервер остаётся отзывчивым.

Разрыв соединения, EOF и истечение срока ожидания сами по себе не отменяют
принятую фоновую работу. Отмена одного потребителя не останавливает
[общую работу](../runtime/shared-work.md), нужную другим.
[Доставка движка](../distribution/shared-engine-delivery.md) принадлежит
процессу: отменяется ожидание вызова, а сама доставка продолжается.

Уже выполненные внешние изменения автоматически не откатываются.
При этом отмена не ослабляет гарантии согласованности записи и восстановления.
В MCP → daemon явная отмена отделена от SDK-сигнала завершения запроса.
До отправки предметной операции отмена подтверждается по её точному ключу;
неизвестный исход управляющего обмена не разрешает повторное исполнение.
Начатый ответ дописывается до фактического flush или закрытия транспорта:
отмена не оставляет частичный JSON и не освобождает владельца раньше времени.

В современном протоколе полученный полный ответ позволяет повторно использовать
идентификатор запроса. Новая отмена адресует новый вызов; прежний владелец
сохраняется до завершения своего исполнения, ответа и уже принятого управления.
Пока отслеживаемый `tools/call` не получил ответа, другой запрос с его
идентификатором его не подменяет.

Проверки связывают настоящие MCP-уведомления с TCP-демоном, сохранёнными
квитанциями, двумя потребителями общего индекса и фактическим созданием базы.
На полном маршруте `run` уход единственного ожидающего вызова не прерывает
доставку: движок устанавливается по закреплённой SHA-256, а последующий вызов
использует эту установку без повторного скачивания.

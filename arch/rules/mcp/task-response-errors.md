---
id: INV.WIRE.TASK-RESPONSE-ERRORS
check:
  - crates/unica-coder/src/infrastructure/daemon/client_v5.rs::task_exchange_reports_complete_malformed_frame_as_protocol_and_closes_the_session
  - crates/unica-coder/src/infrastructure/daemon/client_v5.rs::task_exchange_reports_connection_lost_before_a_complete_frame_as_transport
  - crates/unica-coder/src/infrastructure/daemon/client_v5.rs::task_exchange_post_parse_expiry_wins_for_valid_and_malformed_frames
  - crates/unica-coder/src/interfaces/mcp.rs::task_reads_report_complete_malformed_daemon_frames_as_protocol_failures
  - crates/unica-coder/src/interfaces/mcp.rs::task_reads_report_unreachable_or_truncated_daemon_as_transport_failures
---

# Некорректный ответ о задании отличается от обрыва связи

При запросе состояния или результата задания ошибка установления соединения
или обрыв до получения полного ответа дают `task_transport_failed`.
Если срок запроса не истёк, но полученный полный ответ нарушает протокол,
Unica возвращает `task_protocol_failed`. Например, статус `completed`
без обязательного результата — ошибка ответа, а не связи.

Некорректное содержимое не выдаётся вызывающему, испорченное соединение
не используется повторно. Ошибка обмена не запускает задание заново.

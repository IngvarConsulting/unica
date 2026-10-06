---
id: INV.RUNTIME.SERVICE-OPERATION-CANCELLATION
check:
  - crates/unica-coder/src/infrastructure/workspace_services.rs::manager_generates_unique_uuid_operation_ids_for_bsl_and_rlm
  - crates/unica-coder/src/infrastructure/workspace_services.rs::cancellable_connector_sends_cancel_on_a_separate_connection
  - crates/unica-coder/src/infrastructure/workspace_services.rs::workspace_service_detached_caller_keeps_work_and_exact_cancel_ownership
  - crates/unica-coder/src/infrastructure/workspace_services.rs::workspace_service_half_closed_caller_receives_late_success_without_cancel
  - crates/unica-coder/src/infrastructure/workspace_services.rs::workspace_service_monitor_failures_preserve_worker_and_report_real_error_after_completion
  - crates/unica-coder/src/infrastructure/workspace_services.rs::workspace_service_control_path_drains_disconnected_worker_before_cleanup
---

# Отмена внутреннего запроса адресует только его операцию

Рабочий запрос внутреннего workspace-сервиса получает уникальный UUID
операции. Отмена передаётся по отдельному соединению с этим UUID, поэтому
не ждёт ответа выполняемой работы. Разрыв рабочего соединения сам по себе
не отменяет уже принятую операцию. Её идентификатор и владелец сохраняются
до фактического завершения; явная отмена по прежнему UUID по-прежнему
адресует только эту операцию.

Закрытие отправки клиентом не доказывает невозможность чтения ответа:
helper пытается передать фактически полученный результат. Ошибка наблюдения
или доставки остаётся ошибкой транспорта и не превращается в отмену работы.
Остановка сервиса явно отменяет работу по
[правилу остановки](service-shutdown.md).

Это внутренний канал менеджера и helper-сервиса; он сохраняет различие
отмены и разрыва соединения по
[общему правилу](../mcp/explicit-call-cancellation.md).
Проверки проходят менеджер, TCP-коннектор и helper-сервис с управляемым
исполнителем, включая настоящий TCP EOF и закрытие только отправки.

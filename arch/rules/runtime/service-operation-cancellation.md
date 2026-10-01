---
id: INV.RUNTIME.SERVICE-OPERATION-CANCELLATION
check:
  - crates/unica-coder/src/infrastructure/workspace_services.rs::manager_generates_unique_uuid_operation_ids_for_bsl_and_rlm
  - crates/unica-coder/src/infrastructure/workspace_services.rs::cancellable_connector_sends_cancel_on_a_separate_connection
  - crates/unica-coder/src/infrastructure/workspace_services.rs::workspace_service_control_path_disconnect_cancels_only_its_operation
---

# Отмена внутреннего запроса адресует только его операцию

Рабочий запрос внутреннего workspace-сервиса получает уникальный UUID
операции. Отмена передаётся по отдельному соединению с этим UUID, поэтому
не ждёт ответа выполняемой работы. Разрыв рабочего соединения останавливает
связанную с ним операцию сервиса.

Это внутренний канал менеджера и helper-сервиса. Правило не означает отмену
задания демона при отключении публичного MCP-клиента. Проверки проходят
менеджер, TCP-коннектор и helper-сервис с управляемым исполнителем.

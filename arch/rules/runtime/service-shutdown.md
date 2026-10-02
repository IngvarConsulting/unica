---
id: INV.RUNTIME.SERVICE-SHUTDOWN
check:
  - crates/unica-coder/src/infrastructure/workspace_services.rs::workspace_service_control_path_shutdown_cancels_all_and_rejects_new_work
---

# Остановка внутреннего сервиса отменяет принятую работу

Получив `shutdown`, внутренний сервис отменяет зарегистрированные операции
и больше не принимает новую работу. Завершение очищает реестр операций.
Проверка проходит TCP-сервис с управляемым исполнителем.

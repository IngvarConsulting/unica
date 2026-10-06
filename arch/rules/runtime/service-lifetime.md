---
id: INV.RUNTIME.SERVICE-LIFETIME
check:
  - crates/unica-coder/src/infrastructure/workspace_services.rs::workspace_service_never_stops_under_a_request_longer_than_its_idle_time
  - crates/unica-coder/src/infrastructure/workspace_services.rs::service_config_uses_defaults_and_env_overrides
  - crates/unica-coder/src/infrastructure/workspace_services.rs::workspace_service_control_path_shutdown_cancels_all_and_rejects_new_work
---

# Внутренний сервис живёт, пока не простаивает

Workspace helper завершает работу после простоя: по умолчанию 7200 секунд,
значение меняет переменная `UNICA_WORKSPACE_SERVICE_IDLE_SECS`. Предельного
возраста у сервиса нет. Выполняющийся запрос не считается простоем, поэтому
сервис не останавливается под ним, сколько бы запрос ни длился. Остановка
проходит через отмену зарегистрированной работы.

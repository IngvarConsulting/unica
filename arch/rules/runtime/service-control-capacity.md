---
id: INV.RUNTIME.SERVICE-CONTROL-CAPACITY
check:
  - crates/unica-coder/tests/platform/workspace_services_integration.rs::workspace_service_admission_ninth_work_runs_with_exact_cancel_ping_and_shutdown
  - crates/unica-coder/tests/platform/workspace_services_integration.rs::workspace_service_admission_sixty_fifth_partial_header_reaches_handler_and_executes_work
  - crates/unica-coder/tests/platform/workspace_services_integration.rs::workspace_service_admission_ninth_control_delivers_exact_cancel_with_all_old_slots_held
  - crates/unica-coder/tests/platform/workspace_services_integration.rs::slow_general_headers_cannot_exhaust_control_capacity
  - crates/unica-coder/tests/platform/workspace_services_integration.rs::workspace_service_concurrent_work_preserves_control_path
---

# Занятые обработчики не блокируют приём и управление сервисом

Число рабочих, общих или управляющих обработчиков само по себе не отклоняет
новую работу, проверку доступности, отмену или остановку внутреннего сервиса.
Принятое соединение обслуживается без численной квоты обработчиков;
выполняемая работа не блокирует управление сервисом. Незавершённые заголовки
других соединений не закрывают доставку отмены точной операции.

Проверки токена и уникальности идентификатора операции сохраняются.
Явная остановка отменяет зарегистрированную работу и закрывает новый приём
по [правилу остановки сервиса](service-shutdown.md). Реальный отказ ОС создать
поток отличается от искусственной квоты.
